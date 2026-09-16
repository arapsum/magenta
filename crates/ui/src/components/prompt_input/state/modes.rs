use super::*;

impl PromptComposer {
    fn has_content(&self, cx: &App) -> bool {
        !self.input.read(cx).value().trim().is_empty() || !self.attachments.is_empty()
    }

    pub(crate) fn is_ready(&self, cx: &App) -> bool {
        let workspace_ready = self.mode == ConversationMode::Chat
            || (self.agent_capability.available()
                && self
                    .workspace_root
                    .as_deref()
                    .is_some_and(std::path::Path::is_dir));
        self.storage_ready
            && (self.retry_target.is_some() || self.has_content(cx))
            && self.model.is_some()
            && self.effort.is_some()
            && workspace_ready
    }

    pub(crate) fn select_mode(
        &mut self,
        mode: ConversationMode,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if mode == ConversationMode::Agent && !self.agent_capability.available() {
            return;
        }

        if self.mode == mode && self.input_mode == mode {
            return;
        }

        self.mode = mode;
        self.synchronize_input_mode(window, cx);
    }

    pub(crate) fn synchronize_input_mode(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.input_mode == self.mode {
            return;
        }

        let current = ModeDraft {
            prompt: self.input.read(cx).value().to_string(),
            attachments: std::mem::take(&mut self.attachments),
        };

        match self.input_mode {
            ConversationMode::Chat => self.chat_draft = current,
            ConversationMode::Agent => self.work_draft = current,
        }

        let mode = self.mode.clone();
        let next = match &mode {
            ConversationMode::Chat => &mut self.chat_draft,
            ConversationMode::Agent => &mut self.work_draft,
        };

        self.input.update(cx, |input, cx| {
            input.set_value(&next.prompt, window, cx);
            input.set_placeholder(
                if mode == ConversationMode::Chat {
                    "Ask Magenta anything…"
                } else {
                    "What should we work on?"
                },
                window,
                cx,
            );
        });

        self.attachments = std::mem::take(&mut next.attachments);
        self.input_mode = mode;
        self.schedule_code_preview(window, cx);
        cx.notify();
    }

    pub(crate) fn open_workspace_panel(
        &self,
        panel: PromptWorkspacePanel,
        cx: &mut Context<'_, Self>,
    ) {
        if self.mode == ConversationMode::Agent && self.workspace_root.is_some() {
            cx.emit(PromptComposerEvent::OpenWorkspacePanel(panel));
        }
    }

    pub(crate) fn choose_workspace(&mut self, window: &Window, cx: &Context<'_, Self>) {
        if self.mode != ConversationMode::Agent || !self.agent_capability.available() {
            return;
        }

        let picker = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Choose workspace".into()),
        });
        self.workspace_task = Some(cx.spawn_in(window, async move |composer, window| {
            let selection = match picker.await {
                Ok(Ok(paths)) => paths,
                Ok(Err(source)) => {
                    _ = composer.update_in(window, |composer, _window, cx| {
                        composer.workspace_task = None;
                        let error = MagentaError::AttachmentPicker { source };
                        composer.set_inline_error(error.presentation(), cx);
                    });
                    return;
                }
                Err(_) => return,
            };
            _ = composer.update_in(window, |composer, _, cx| {
                composer.workspace_task = None;
                composer.workspace_root = selection
                    .and_then(|paths| paths.into_iter().next())
                    .filter(|path| path.is_dir());
                if let Some(root) = composer.workspace_root.clone() {
                    cx.emit(PromptComposerEvent::WorkspaceSelected(root));
                }
                cx.notify();
            });
        }));
    }

    pub(crate) fn select_model(&mut self, model: ModelDescriptor, cx: &mut Context<'_, Self>) {
        if self.model.as_ref() != Some(&model) {
            self.effort = Some(model.default_effort.clone());
            self.model = Some(model);
            cx.notify();
        }
    }

    pub(crate) fn select_effort(&mut self, effort: EffortLevel, cx: &mut Context<'_, Self>) {
        if self
            .model
            .as_ref()
            .is_some_and(|model| model.supported_efforts.contains(&effort))
            && self.effort.as_ref() != Some(&effort)
        {
            self.effort = Some(effort);
            cx.notify();
        }
    }

    pub(crate) fn request(&self, cx: &App) -> Option<PromptRequest> {
        if !self.is_ready(cx) {
            return None;
        }
        let model = self.model.as_ref()?;
        let effort = self.effort.clone()?;
        Some(PromptRequest {
            prompt: self.input.read(cx).value().trim().to_owned().into(),
            generation: GenerationConfig::new(model.provider.clone(), model.id.clone(), effort)
                .with_limits(model.limits),
            attachments: if self.mode == ConversationMode::Chat {
                self.attachments
                    .iter()
                    .map(|attachment| attachment.path.clone())
                    .collect()
            } else {
                Vec::new()
            },
            mode: self.mode.clone(),
            workspace_root: self.workspace_root.clone(),
        })
    }

    pub(crate) fn submit(&self, cx: &mut Context<'_, Self>) {
        if !self.generating
            && let Some(request) = self.request(cx)
        {
            cx.emit(PromptComposerEvent::Submit(request));
        }
    }

    pub(crate) fn cancel(&self, cx: &mut Context<'_, Self>) {
        if self.generating {
            cx.emit(PromptComposerEvent::Cancel);
        }
    }
}
