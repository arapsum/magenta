use magenta_application::resolve_generation_defaults;

use super::*;

impl PromptComposer {
    const fn generation_state(&self, mode: &ConversationMode) -> &ModeGenerationState {
        match mode {
            ConversationMode::Chat => &self.chat_generation,
            ConversationMode::Agent => &self.work_generation,
        }
    }

    const fn generation_state_mut(&mut self, mode: &ConversationMode) -> &mut ModeGenerationState {
        match mode {
            ConversationMode::Chat => &mut self.chat_generation,
            ConversationMode::Agent => &mut self.work_generation,
        }
    }

    pub(crate) fn save_active_generation(&mut self) {
        let mode = self.input_mode.clone();
        let model = self.model.clone();
        let effort = self.effort.clone();
        let state = self.generation_state_mut(&mode);
        state.model = model;
        state.effort = effort;
    }

    pub(crate) fn restore_generation_for_input_mode(&mut self) {
        let mode = self.input_mode.clone();
        let (model, effort) = {
            let state = self.generation_state(&mode);
            (state.model.clone(), state.effort.clone())
        };
        self.model = model;
        self.effort = effort;
    }

    const fn active_generation_origin(&self) -> GenerationSelectionOrigin {
        self.generation_state(&self.input_mode).origin
    }

    pub(crate) fn active_generation_is_automatic(&self) -> bool {
        self.active_generation_origin() == GenerationSelectionOrigin::Automatic
    }

    pub(crate) const fn active_model_available(&self) -> bool {
        self.generation_state(&self.input_mode).model_available
    }

    pub(crate) const fn active_effort_available(&self) -> bool {
        self.generation_state(&self.input_mode).effort_available
    }

    pub(crate) fn active_persisted_generation(&self) -> Option<GenerationConfig> {
        self.generation_state(&self.input_mode).persisted.clone()
    }

    fn clear_catalog_state(state: &mut ModeGenerationState) {
        if let Some(configuration) = state.persisted.as_ref() {
            state.model = Some(Self::synthetic_model(configuration));
            state.effort = Some(configuration.effort.clone());
        } else {
            state.model = None;
            state.effort = None;
        }
        state.model_available = false;
        state.effort_available = false;
    }

    fn synthetic_model(configuration: &GenerationConfig) -> ModelDescriptor {
        ModelDescriptor {
            provider: configuration.provider.clone(),
            id: configuration.model.clone(),
            display_name: configuration.model.0.clone(),
            description: None,
            priority: 0,
            default_effort: configuration.effort.clone(),
            supported_efforts: vec![configuration.effort.clone()],
            limits: configuration.limits,
        }
    }

    fn apply_default_for_mode(&mut self, mode: &ConversationMode) {
        let configured = self.generation_settings.for_mode(mode.clone()).is_some();
        let resolution =
            resolve_generation_defaults(mode.clone(), &self.generation_settings, &self.models);
        let resolved_model = resolution.config.as_ref().and_then(|configuration| {
            self.models
                .iter()
                .find(|model| {
                    if model.provider != configuration.provider {
                        return false;
                    }
                    model.id == configuration.model
                })
                .cloned()
        });
        let state = self.generation_state_mut(mode);
        state.origin = if configured {
            GenerationSelectionOrigin::ConfiguredDefault
        } else {
            GenerationSelectionOrigin::Automatic
        };
        state.persisted = None;

        let Some(configuration) = resolution.config else {
            Self::clear_catalog_state(state);
            return;
        };

        state.model = resolved_model.or_else(|| Some(Self::synthetic_model(&configuration)));
        state.effort = Some(configuration.effort);
        state.model_available = true;
        state.effort_available = true;
    }

    fn apply_catalog_to_manual_or_persisted(&mut self, mode: &ConversationMode) {
        let (requested_provider, requested_model, requested_effort, origin) = {
            let state = self.generation_state(mode);
            let requested = state.persisted.as_ref().map_or_else(
                || {
                    state.model.as_ref().map(|model| {
                        (
                            model.provider.clone(),
                            model.id.clone(),
                            state
                                .effort
                                .clone()
                                .unwrap_or_else(|| model.default_effort.clone()),
                        )
                    })
                },
                |configuration| {
                    Some((
                        configuration.provider.clone(),
                        configuration.model.clone(),
                        configuration.effort.clone(),
                    ))
                },
            );
            let Some((provider, model, effort)) = requested else {
                return;
            };
            (provider, model, effort, state.origin)
        };

        let live_model = self
            .models
            .iter()
            .find(|model| model.provider == requested_provider && model.id == requested_model)
            .cloned();
        let state = self.generation_state_mut(mode);

        match live_model {
            Some(model) => {
                state.model = Some(model.clone());
                state.effort = Some(requested_effort.clone());
                state.model_available = true;
                state.effort_available = model.supported_efforts.contains(&requested_effort);
            }
            None if origin == GenerationSelectionOrigin::PersistedConversation => {
                let configuration = state
                    .persisted
                    .clone()
                    .expect("persisted generation state has its configuration");
                state.model = Some(Self::synthetic_model(&configuration));
                state.effort = Some(configuration.effort);
                state.model_available = false;
                state.effort_available = false;
            }
            None => {
                let configuration =
                    GenerationConfig::new(requested_provider, requested_model, requested_effort);
                state.model = Some(Self::synthetic_model(&configuration));
                state.effort = Some(configuration.effort);
                state.model_available = false;
                state.effort_available = false;
            }
        }
    }

    pub(crate) fn set_models(&mut self, models: Vec<ModelDescriptor>, cx: &mut Context<'_, Self>) {
        self.save_active_generation();
        self.models = models;
        self.model_catalog_state = ModelCatalogState::Loaded;

        if self.models.is_empty() {
            Self::clear_catalog_state(&mut self.chat_generation);
            Self::clear_catalog_state(&mut self.work_generation);
        } else {
            for mode in [&ConversationMode::Chat, &ConversationMode::Agent] {
                let origin = self.generation_state(mode).origin;
                if matches!(
                    origin,
                    GenerationSelectionOrigin::Automatic
                        | GenerationSelectionOrigin::ConfiguredDefault
                ) {
                    self.apply_default_for_mode(mode);
                } else {
                    self.apply_catalog_to_manual_or_persisted(mode);
                }
            }
        }

        self.restore_generation_for_input_mode();
        cx.notify();
    }

    pub(crate) fn clear_models(&mut self, cx: &mut Context<'_, Self>) {
        self.save_active_generation();
        self.models.clear();
        self.model_catalog_state = ModelCatalogState::NotLoaded;
        Self::clear_catalog_state(&mut self.chat_generation);
        Self::clear_catalog_state(&mut self.work_generation);
        self.restore_generation_for_input_mode();
        cx.notify();
    }

    pub(crate) fn set_generation_settings(
        &mut self,
        settings: GenerationSettings,
        cx: &mut Context<'_, Self>,
    ) {
        self.save_active_generation();
        self.generation_settings = settings;

        for mode in [&ConversationMode::Chat, &ConversationMode::Agent] {
            let origin = self.generation_state(mode).origin;
            if matches!(
                origin,
                GenerationSelectionOrigin::Automatic | GenerationSelectionOrigin::ConfiguredDefault
            ) {
                self.apply_default_for_mode(mode);
            }
        }

        self.restore_generation_for_input_mode();
        cx.notify();
    }

    pub(crate) fn reset_for_new_conversation(&mut self, cx: &mut Context<'_, Self>) {
        self.chat_generation = ModeGenerationState::default();
        self.work_generation = ModeGenerationState::default();
        self.chat_draft.command_id = None;
        self.work_draft.command_id = None;
        self.selected_command = None;

        if !self.models.is_empty() {
            self.apply_default_for_mode(&ConversationMode::Chat);
            self.apply_default_for_mode(&ConversationMode::Agent);
        }

        self.restore_generation_for_input_mode();
        cx.notify();
    }

    pub(crate) fn set_persisted_generation(
        &mut self,
        configuration: &GenerationConfig,
        cx: &mut Context<'_, Self>,
    ) {
        self.save_active_generation();
        let mode = self.mode.clone();
        let live_model = self
            .models
            .iter()
            .find(|model| {
                if model.provider != configuration.provider {
                    return false;
                }
                model.id == configuration.model
            })
            .cloned();
        let state = self.generation_state_mut(&mode);
        state.origin = GenerationSelectionOrigin::PersistedConversation;
        state.persisted = Some(configuration.clone());
        state.model = live_model
            .clone()
            .or_else(|| Some(Self::synthetic_model(configuration)));
        state.effort = Some(configuration.effort.clone());
        state.model_available = live_model.is_some();
        state.effort_available = live_model
            .as_ref()
            .is_some_and(|model| model.supported_efforts.contains(&configuration.effort));
        self.restore_generation_for_input_mode();
        cx.notify();
    }

    pub(crate) fn select_automatic(&mut self, cx: &mut Context<'_, Self>) {
        self.save_active_generation();
        let mode = self.input_mode.clone();
        {
            let state = self.generation_state_mut(&mode);
            state.origin = GenerationSelectionOrigin::Automatic;
            state.persisted = None;
        }
        if self.models.is_empty() {
            Self::clear_catalog_state(self.generation_state_mut(&mode));
        } else {
            self.apply_default_for_mode(&mode);
        }
        self.restore_generation_for_input_mode();
        cx.notify();
    }

    pub(crate) fn select_manual_model(
        &mut self,
        model: ModelDescriptor,
        cx: &mut Context<'_, Self>,
    ) {
        self.save_active_generation();
        let mode = self.input_mode.clone();
        let state = self.generation_state_mut(&mode);
        state.origin = GenerationSelectionOrigin::Manual;
        state.persisted = None;
        state.model = Some(model.clone());
        state.effort = Some(model.default_effort);
        state.model_available = true;
        state.effort_available = true;
        self.restore_generation_for_input_mode();
        cx.notify();
    }

    pub(crate) fn select_manual_effort(&mut self, effort: EffortLevel, cx: &mut Context<'_, Self>) {
        self.save_active_generation();
        let mode = self.input_mode.clone();
        let state = self.generation_state_mut(&mode);
        if state
            .model
            .as_ref()
            .is_some_and(|model| model.supported_efforts.contains(&effort))
        {
            state.origin = GenerationSelectionOrigin::Manual;
            state.persisted = None;
            state.effort = Some(effort);
            state.effort_available = true;
        }
        self.restore_generation_for_input_mode();
        cx.notify();
    }
}
