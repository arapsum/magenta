use gpui_kit::{App, Context, Window};
use magenta_application::resolve_generation_defaults;
use magenta_core::ConversationMode;

use super::{AccountState, AgentCapability, BubblewrapCapability, MainView, ModelCatalogState};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SetupItemState {
    Loading,
    Ready,
    Attention,
    Unavailable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SetupPrimaryAction {
    Connect,
    ReloadModels,
    Finish,
    CheckingAccount,
    LoadingModels,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SetupItem {
    pub(crate) title: &'static str,
    pub(crate) detail: String,
    pub(crate) state: SetupItemState,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SetupReadiness {
    pub(crate) acknowledged: bool,
    pub(crate) chat_ready: bool,
    pub(crate) primary_action: SetupPrimaryAction,
    pub(crate) account: SetupItem,
    pub(crate) models: SetupItem,
    pub(crate) workspace: SetupItem,
    pub(crate) file_tools: SetupItem,
    pub(crate) commands: SetupItem,
    pub(crate) bubblewrap: SetupItem,
    pub(crate) saving: bool,
    pub(crate) error: Option<crate::ErrorPresentation>,
}

const SETUP_SAVE_ERROR: crate::ErrorPresentation = crate::ErrorPresentation {
    code: "MAG-SETUP-SAVE",
    severity: crate::ErrorSeverity::Error,
    title: "Setup preference could not be saved",
    message: "Magenta kept the setup panel open. Check the settings file and try again.",
};

impl MainView {
    pub(crate) fn setup_readiness(&self, cx: &App) -> SetupReadiness {
        let settings = crate::settings::current(cx);
        let account = account_item(&self.account_state);
        let models = model_item(self.model_catalog_state, &self.models, &settings);
        let chat_ready = matches!(self.account_state, AccountState::Connected(_))
            && self.model_catalog_state.is_loaded()
            && resolve_generation_defaults(
                ConversationMode::Chat,
                &settings.generation,
                &self.models,
            )
            .config
            .is_some();
        let composer = self.composer.read(cx);
        let capability = composer.agent_capability();
        let workspace = composer.workspace_root();

        SetupReadiness {
            acknowledged: settings.onboarding.is_setup_acknowledged(),
            chat_ready,
            primary_action: match &self.account_state {
                AccountState::Restoring | AccountState::WaitingForBrowser => {
                    SetupPrimaryAction::CheckingAccount
                }
                AccountState::SignedOut | AccountState::Failed(_) => SetupPrimaryAction::Connect,
                AccountState::Connected(_) => match self.model_catalog_state {
                    ModelCatalogState::Loading => SetupPrimaryAction::LoadingModels,
                    ModelCatalogState::Loaded if chat_ready => SetupPrimaryAction::Finish,
                    ModelCatalogState::NotLoaded
                    | ModelCatalogState::Loaded
                    | ModelCatalogState::Failed(_) => SetupPrimaryAction::ReloadModels,
                },
            },
            account,
            models,
            workspace: workspace.map_or_else(
                || SetupItem {
                    title: "Workspace",
                    detail: "Choose a project when you switch to Work.".to_owned(),
                    state: SetupItemState::Attention,
                },
                |root| SetupItem {
                    title: "Workspace",
                    detail: root.display().to_string(),
                    state: SetupItemState::Ready,
                },
            ),
            file_tools: match capability {
                AgentCapability::Unavailable => SetupItem {
                    title: "File tools",
                    detail: "Work tools are unavailable in this build.".to_owned(),
                    state: SetupItemState::Unavailable,
                },
                AgentCapability::Files | AgentCapability::Commands => SetupItem {
                    title: "File tools",
                    detail: "Project-scoped reading and reviewed edits are available.".to_owned(),
                    state: SetupItemState::Ready,
                },
            },
            commands: match capability {
                AgentCapability::Commands => SetupItem {
                    title: "Commands",
                    detail: "Sandboxed project commands are available.".to_owned(),
                    state: SetupItemState::Ready,
                },
                AgentCapability::Files => SetupItem {
                    title: "Commands",
                    detail: "Unavailable until the reviewed workspace can be mounted safely."
                        .to_owned(),
                    state: SetupItemState::Unavailable,
                },
                AgentCapability::Unavailable => SetupItem {
                    title: "Commands",
                    detail: "Work tools are unavailable in this build.".to_owned(),
                    state: SetupItemState::Unavailable,
                },
            },
            bubblewrap: match self.work_runtime.bubblewrap() {
                BubblewrapCapability::Available => SetupItem {
                    title: "Bubblewrap",
                    detail: "Installed and able to start an isolated process.".to_owned(),
                    state: SetupItemState::Ready,
                },
                BubblewrapCapability::Unavailable => SetupItem {
                    title: "Bubblewrap",
                    detail: "Not installed or unable to start an isolated process.".to_owned(),
                    state: SetupItemState::Unavailable,
                },
            },
            saving: self.setup_save_task.is_some(),
            error: self.setup_error,
        }
    }

    pub(crate) fn choose_setup_workspace(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.composer.update(cx, |composer, cx| {
            composer.select_mode(ConversationMode::Agent, window, cx);
            composer.choose_workspace(window, cx);
        });
    }

    pub(crate) fn sync_settings_setup(&self, cx: &mut Context<'_, Self>) {
        let Some(settings_view) = self.settings_view.as_ref() else {
            return;
        };
        let setup = self.setup_readiness(cx);
        settings_view.update(cx, |settings, cx| settings.set_setup(setup, cx));
    }

    pub(crate) fn acknowledge_setup(
        &mut self,
        focus_composer: bool,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.persist_setup_acknowledgement(true, focus_composer, window, cx);
    }

    pub(crate) fn show_setup_on_new_chat(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.persist_setup_acknowledgement(false, false, window, cx);
    }

    fn persist_setup_acknowledgement(
        &mut self,
        acknowledged: bool,
        focus_composer: bool,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.setup_save_task.is_some() {
            return;
        }

        let mut next = crate::settings::current(cx);
        next.onboarding.set_setup_acknowledged(acknowledged);
        let store = Arc::clone(&self.settings_store);
        self.setup_error = None;
        self.setup_save_task = Some(cx.spawn_in(window, async move |view, window| {
            let result = store.save(next.clone()).await;
            _ = view.update_in(window, |main, window, cx| {
                main.setup_save_task = None;
                match result {
                    Ok(()) => {
                        crate::settings::replace(next, cx);
                        main.setup_error = None;
                        if focus_composer {
                            main.composer.update(cx, |composer, cx| {
                                composer.focus(window, cx);
                            });
                        }
                    }
                    Err(error) => {
                        tracing::warn!(
                            error = %error.source,
                            operation = "setup.save",
                            "could not save setup acknowledgement"
                        );
                        main.setup_error = Some(SETUP_SAVE_ERROR);
                    }
                }
                main.sync_settings_setup(cx);
                cx.notify();
            });
        }));
        self.sync_settings_setup(cx);
        cx.notify();
    }
}

fn account_item(state: &AccountState) -> SetupItem {
    match state {
        AccountState::Restoring => SetupItem {
            title: "ChatGPT account",
            detail: "Checking the saved sign-in…".to_owned(),
            state: SetupItemState::Loading,
        },
        AccountState::SignedOut => SetupItem {
            title: "ChatGPT account",
            detail: "Connect your account to load available models.".to_owned(),
            state: SetupItemState::Attention,
        },
        AccountState::WaitingForBrowser => SetupItem {
            title: "ChatGPT account",
            detail: "Finish signing in through your browser.".to_owned(),
            state: SetupItemState::Loading,
        },
        AccountState::Connected(account) => SetupItem {
            title: "ChatGPT account",
            detail: account
                .email
                .clone()
                .unwrap_or_else(|| "Connected".to_owned()),
            state: SetupItemState::Ready,
        },
        AccountState::Failed(error) => SetupItem {
            title: "ChatGPT account",
            detail: format!("{} Reference: {}", error.message, error.code),
            state: SetupItemState::Attention,
        },
    }
}

fn model_item(
    state: ModelCatalogState,
    models: &[magenta_core::ModelDescriptor],
    settings: &magenta_core::AppSettings,
) -> SetupItem {
    match state {
        ModelCatalogState::NotLoaded => SetupItem {
            title: "Chat model",
            detail: "Models load after your account is connected.".to_owned(),
            state: SetupItemState::Unavailable,
        },
        ModelCatalogState::Loading => SetupItem {
            title: "Chat model",
            detail: "Loading available models…".to_owned(),
            state: SetupItemState::Loading,
        },
        ModelCatalogState::Loaded => {
            let resolution =
                resolve_generation_defaults(ConversationMode::Chat, &settings.generation, models);
            resolution.config.map_or_else(
                || SetupItem {
                    title: "Chat model",
                    detail: "No usable models are available for this account.".to_owned(),
                    state: SetupItemState::Attention,
                },
                |configuration| SetupItem {
                    title: "Chat model",
                    detail: format!(
                        "{} · {} effort",
                        configuration.model.0,
                        configuration.effort.label()
                    ),
                    state: SetupItemState::Ready,
                },
            )
        }
        ModelCatalogState::Failed(error) => SetupItem {
            title: "Chat model",
            detail: format!("{} Reference: {}", error.message, error.code),
            state: SetupItemState::Attention,
        },
    }
}

use std::sync::Arc;
