use gpui_kit::Context;
use gpui_kit::Window;
use gpui_kit::component::{
    WindowExt as _,
    notification::{Notification, NotificationType},
};
use magenta_application::{GenerationFallbackReason, resolve_generation_defaults};
use magenta_core::{ConversationMode, ModelDescriptor};

use crate::components::prompt_input::PromptComposer;

use super::{MainView, SettingsWindow};

struct GenerationFallbackNotification;

impl MainView {
    pub(crate) fn set_model_catalog(
        &mut self,
        models: Vec<ModelDescriptor>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.models.clone_from(&models);
        self.model_catalog_loaded = true;
        self.composer.update(cx, |composer, cx| {
            composer.set_models(models.clone(), cx);
        });
        if let Some(settings_view) = self.settings_view.as_ref() {
            settings_view.update(cx, |settings, cx| settings.set_models(models, cx));
        }
        self.apply_generation_defaults(window, cx);
        cx.notify();
    }

    pub(crate) fn clear_model_catalog(&mut self, cx: &mut Context<'_, Self>) {
        self.models.clear();
        self.model_catalog_loaded = false;
        self.chat_fallback_key = None;
        self.work_fallback_key = None;
        self.composer.update(cx, PromptComposer::clear_models);
        if let Some(settings_view) = self.settings_view.as_ref() {
            settings_view.update(cx, SettingsWindow::clear_models);
        }
        cx.notify();
    }

    pub(crate) fn apply_generation_defaults(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let settings = crate::settings::current(cx);
        self.composer.update(cx, |composer, cx| {
            composer.set_generation_settings(settings.generation.clone(), cx);
        });

        if !self.model_catalog_loaded {
            self.chat_fallback_key = None;
            self.work_fallback_key = None;
            return;
        }

        for mode in [&ConversationMode::Chat, &ConversationMode::Agent] {
            let resolution =
                resolve_generation_defaults(mode.clone(), &settings.generation, &self.models);
            self.update_fallback_notice(mode, resolution.fallback.as_ref(), window, cx);
        }
    }

    fn update_fallback_notice(
        &mut self,
        mode: &ConversationMode,
        reason: Option<&GenerationFallbackReason>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let key = reason.map(|reason| fallback_key(mode, reason));
        let stored_key = match mode {
            ConversationMode::Chat => &mut self.chat_fallback_key,
            ConversationMode::Agent => &mut self.work_fallback_key,
        };

        if stored_key.as_ref() == key.as_ref() {
            return;
        }

        if let Some(reason) = reason {
            window.push_notification(
                Notification::new()
                    .id1::<GenerationFallbackNotification>(
                        key.clone()
                            .unwrap_or_else(|| "generation-fallback".to_owned()),
                    )
                    .title("Generation default adjusted")
                    .message(fallback_message(mode, reason))
                    .with_type(NotificationType::Warning)
                    .autohide(true),
                cx,
            );
        }

        *stored_key = key;
    }
}

fn fallback_key(mode: &ConversationMode, reason: &GenerationFallbackReason) -> String {
    let mode = match mode {
        ConversationMode::Chat => "chat",
        ConversationMode::Agent => "work",
    };
    match reason {
        GenerationFallbackReason::ModelUnavailable {
            requested_provider,
            requested_model,
            effective_provider,
            effective_model,
        }
        | GenerationFallbackReason::ProviderUnavailable {
            requested_provider,
            requested_model,
            effective_provider,
            effective_model,
        } => format!(
            "{mode}:model:{}:{}:{}:{}",
            requested_provider.0, requested_model.0, effective_provider.0, effective_model.0
        ),
        GenerationFallbackReason::EffortUnavailable {
            provider,
            model,
            requested,
            effective,
        } => format!(
            "{mode}:effort:{}:{}:{}:{}",
            provider.0,
            model.0,
            requested.wire_value(),
            effective.wire_value()
        ),
    }
}

fn fallback_message(mode: &ConversationMode, reason: &GenerationFallbackReason) -> String {
    let mode = match mode {
        ConversationMode::Chat => "Chat",
        ConversationMode::Agent => "Work",
    };
    match reason {
        GenerationFallbackReason::ModelUnavailable {
            requested_model,
            effective_model,
            ..
        } => format!(
            "{mode} default model `{}` is unavailable; using `{}` for new conversations.",
            requested_model.0, effective_model.0
        ),
        GenerationFallbackReason::ProviderUnavailable {
            requested_provider,
            effective_provider,
            effective_model,
            ..
        } => format!(
            "{mode} default provider `{}` is unavailable; using `{}` / `{}` for new conversations.",
            requested_provider.0, effective_provider.0, effective_model.0
        ),
        GenerationFallbackReason::EffortUnavailable {
            model,
            requested,
            effective,
            ..
        } => format!(
            "{mode} default effort `{}` is unavailable for `{}`; using `{}`.",
            requested.label(),
            model.0,
            effective.label()
        ),
    }
}
