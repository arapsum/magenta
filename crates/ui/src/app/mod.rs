mod account;
mod deletion;
mod finder;
mod generation;
mod history;
mod projects;
mod render;
mod runs;
mod settings_window;
pub(crate) mod setup;
#[cfg(test)]
#[path = "../../test/app/mod.rs"]
mod tests;

use std::{sync::Arc, time::Duration};

use gpui_kit::component::{
    ActiveTheme as _, FocusTrapElement as _, Icon, IconName, Selectable as _, Sizable as _,
    StyledExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputEvent, InputState},
    scroll::ScrollableElement as _,
    v_flex,
};
use gpui_kit::{
    AnyElement, AppContext as _, Context, Entity, FocusHandle, Focusable as _, FontWeight,
    HighlightStyle, KeyBinding, MouseButton, Render, Role, SharedString,
    StatefulInteractiveElement as _, StyledText, Subscription, Task, Window, WindowHandle, div,
    prelude::*, px,
};
use magenta_application::{
    ConversationHistory, ProjectCatalog, RegenerateMessage, RunWorkspaceAgent, SendMessage,
};
use magenta_core::{
    CommandCatalog, ConversationId, ConversationSearchResult, MessageId, ModelCatalog,
    ModelDescriptor, ProviderAccount, ProviderAuthenticator, RepositoryAccess, SettingsStore,
};

use self::settings_window::{
    AccountSettingsState, SettingsDestination, SettingsWindow, SettingsWindowEvent,
};
use crate::components::{
    agent_workbench::{AgentWorkbench, AgentWorkbenchEvent, WorkbenchSection},
    conversation::{ConversationView, ConversationViewEvent},
    prompt_input::{AgentCapability, PromptComposer, PromptComposerEvent, PromptWorkspacePanel},
    sidebar::{SidebarEvent, SidebarView},
    titlebar, workspace,
};

#[derive(Clone, Debug, Default, Eq, PartialEq, gpui_kit::Action)]
#[action(namespace = magenta)]
pub struct OpenConversationFinder;

#[derive(Clone, Debug, Default, Eq, PartialEq, gpui_kit::Action)]
#[action(namespace = magenta)]
pub struct CloseConversationFinder;

#[derive(Clone, Debug, Default, Eq, PartialEq, gpui_kit::Action)]
#[action(namespace = magenta)]
struct SelectNextFinderResult;

#[derive(Clone, Debug, Default, Eq, PartialEq, gpui_kit::Action)]
#[action(namespace = magenta)]
struct SelectPreviousFinderResult;

#[derive(Clone, Debug, Default, Eq, PartialEq, gpui_kit::Action)]
#[action(namespace = magenta)]
struct ConfirmFinderResult;

#[derive(Clone, Debug, Default, Eq, PartialEq, gpui_kit::Action)]
#[action(namespace = magenta)]
struct ConfirmConversationDeletion;

#[derive(Clone, Debug, Default, Eq, PartialEq, gpui_kit::Action)]
#[action(namespace = magenta)]
struct CancelConversationDeletion;

#[cfg(target_os = "macos")]
const OPEN_FINDER_KEY: &str = "cmd-k";
#[cfg(not(target_os = "macos"))]
const OPEN_FINDER_KEY: &str = "ctrl-k";

pub struct MainView {
    sidebar: Entity<SidebarView>,
    composer: Entity<PromptComposer>,
    conversation: Entity<ConversationView>,
    workbench: Option<Entity<AgentWorkbench>>,
    workbench_open: bool,
    send_message: SendMessage,
    agent: Option<RunWorkspaceAgent>,
    regenerate_message: RegenerateMessage,
    retry_target: Option<MessageId>,
    authenticator: Arc<dyn ProviderAuthenticator>,
    model_catalog: Arc<dyn ModelCatalog>,
    models: Vec<ModelDescriptor>,
    model_catalog_state: ModelCatalogState,
    chat_fallback_key: Option<String>,
    work_fallback_key: Option<String>,
    response_runs: runs::ResponseRunCoordinator,
    history: ConversationHistory,
    projects: Option<ProjectCatalog>,
    work_runtime: WorkRuntimeReadiness,
    storage_ready: StorageState,
    history_error: Option<crate::ErrorPresentation>,
    operation: history::Operation,
    operation_task: Option<Task<()>>,
    history_task: Option<Task<()>>,
    load_task: Option<Task<()>>,
    page_task: Option<Task<()>>,
    title_task: Option<Task<()>>,
    load_generation: u64,
    loading_conversation: Option<ConversationId>,
    deferred_navigation: Option<history::Navigation>,
    close_requested: CloseState,
    active_conversation: Option<ConversationId>,
    pending_deletion: Option<PendingDeletion>,
    account_state: AccountState,
    finder_open: PanelState,
    finder_input: Entity<InputState>,
    finder_focus_handle: FocusHandle,
    finder_return_focus: Option<FocusHandle>,
    finder_selected: usize,
    finder_results: Vec<ConversationSearchResult>,
    finder_search_status: FinderSearchStatus,
    finder_search_generation: u64,
    finder_search_task: Option<Task<()>>,
    focus_handle: FocusHandle,
    account_task: Option<Task<()>>,
    model_task: Option<Task<()>>,
    settings_store: Arc<dyn SettingsStore>,
    settings_load_task: Option<Task<()>>,
    setup_save_task: Option<Task<()>>,
    setup_error: Option<crate::ErrorPresentation>,
    settings_window: Option<WindowHandle<gpui_kit::component::Root>>,
    settings_view: Option<Entity<SettingsWindow>>,
    settings_subscription: Option<Subscription>,
    subscriptions: Vec<Subscription>,
}

struct PendingDeletion {
    id: ConversationId,
    title: String,
    focus_handle: FocusHandle,
}

pub struct MainServices {
    pub authenticator: Arc<dyn ProviderAuthenticator>,
    pub model_catalog: Arc<dyn ModelCatalog>,
    pub command_catalog: Arc<dyn CommandCatalog>,
    pub settings_store: Arc<dyn SettingsStore>,
    pub agent: Option<RunWorkspaceAgent>,
    pub projects: Option<ProjectCatalog>,
    pub repository: Option<Arc<dyn RepositoryAccess>>,
    pub work_runtime: WorkRuntimeReadiness,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BubblewrapCapability {
    Available,
    Unavailable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WorkRuntimeReadiness {
    bubblewrap: BubblewrapCapability,
}

impl WorkRuntimeReadiness {
    #[must_use]
    pub const fn new(bubblewrap: BubblewrapCapability) -> Self {
        Self { bubblewrap }
    }

    #[must_use]
    pub const fn bubblewrap(self) -> BubblewrapCapability {
        self.bubblewrap
    }
}

fn configure_agent_composer(
    composer: &Entity<PromptComposer>,
    agent: Option<&RunWorkspaceAgent>,
    cx: &mut Context<'_, MainView>,
) {
    let capability = agent.map_or(AgentCapability::Unavailable, |agent| {
        if agent.supports_commands() {
            AgentCapability::Commands
        } else {
            AgentCapability::Files
        }
    });
    composer.update(cx, |composer, cx| {
        composer.set_agent_capability(capability, cx);
    });
}

fn bind_main_keys(cx: &mut Context<'_, MainView>) {
    cx.bind_keys([
        KeyBinding::new(OPEN_FINDER_KEY, OpenConversationFinder, None),
        KeyBinding::new("escape", CloseConversationFinder, None),
        KeyBinding::new("down", SelectNextFinderResult, Some("ConversationFinder")),
        KeyBinding::new("up", SelectPreviousFinderResult, Some("ConversationFinder")),
        KeyBinding::new("enter", ConfirmFinderResult, Some("ConversationFinder")),
        KeyBinding::new(
            "enter",
            ConfirmConversationDeletion,
            Some("DeleteConversationDialog"),
        ),
        KeyBinding::new(
            "escape",
            CancelConversationDeletion,
            Some("DeleteConversationDialog"),
        ),
    ]);
}

#[derive(Clone, Debug)]
enum AccountState {
    Restoring,
    SignedOut,
    WaitingForBrowser,
    Connected(ProviderAccount),
    Failed(crate::ErrorPresentation),
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum ModelCatalogState {
    #[default]
    NotLoaded,
    Loading,
    Loaded,
    Failed(crate::ErrorPresentation),
}

impl ModelCatalogState {
    const fn is_loaded(self) -> bool {
        matches!(self, Self::Loaded)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum StorageState {
    #[default]
    Loading,
    Ready,
    Failed,
}

impl StorageState {
    const fn is_ready(self) -> bool {
        matches!(self, Self::Ready)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum CloseState {
    #[default]
    Open,
    Requested,
}

impl CloseState {
    const fn is_requested(self) -> bool {
        matches!(self, Self::Requested)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum PanelState {
    #[default]
    Closed,
    Open,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum FinderSearchStatus {
    #[default]
    Idle,
    Searching,
    Ready,
    Failed,
}

impl PanelState {
    const fn is_open(self) -> bool {
        matches!(self, Self::Open)
    }
}

impl MainView {
    pub fn new(
        send_message: SendMessage,
        regenerate_message: RegenerateMessage,
        history: ConversationHistory,
        services: MainServices,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        let composer = cx.new(|cx| PromptComposer::new(window, cx));
        let sidebar = cx.new(|cx| SidebarView::new(window, cx));
        let conversation = cx.new(|cx| ConversationView::new(composer.clone(), window, cx));
        let workbench = services
            .projects
            .clone()
            .zip(services.repository.clone())
            .map(|(catalog, repository)| {
                cx.new(|cx| AgentWorkbench::new(catalog, repository, window, cx))
            });
        let finder_input = cx.new(|cx| InputState::new(window, cx).placeholder("Search chats…"));
        let finder_focus_handle = cx.focus_handle();
        let focus_handle = cx.focus_handle();
        focus_handle.focus(window, cx);
        bind_main_keys(cx);
        composer.update(cx, |composer, cx| composer.set_storage_ready(false, cx));
        configure_agent_composer(&composer, services.agent.as_ref(), cx);
        composer.update(cx, |composer, cx| {
            composer.set_command_catalog(Arc::clone(&services.command_catalog), cx);
        });
        let subscriptions = Self::subscribe_to_children(
            &composer,
            &sidebar,
            &conversation,
            workbench.as_ref(),
            &finder_input,
            window,
            cx,
        );

        let mut main = Self {
            sidebar,
            composer,
            conversation,
            workbench,
            workbench_open: false,
            send_message,
            agent: services.agent,
            regenerate_message,
            retry_target: None,
            authenticator: services.authenticator,
            model_catalog: services.model_catalog,
            models: Vec::new(),
            model_catalog_state: ModelCatalogState::NotLoaded,
            chat_fallback_key: None,
            work_fallback_key: None,
            history,
            response_runs: runs::ResponseRunCoordinator::default(),
            projects: services.projects,
            work_runtime: services.work_runtime,
            storage_ready: StorageState::Loading,
            history_error: None,
            operation: history::Operation::Idle,
            operation_task: None,
            history_task: None,
            load_task: None,
            page_task: None,
            title_task: None,
            load_generation: 0,
            loading_conversation: None,
            deferred_navigation: None,
            close_requested: CloseState::Open,
            active_conversation: None,
            pending_deletion: None,
            account_state: AccountState::Restoring,
            finder_open: PanelState::Closed,
            finder_input,
            finder_focus_handle,
            finder_return_focus: None,
            finder_selected: 0,
            finder_results: Vec::new(),
            finder_search_status: FinderSearchStatus::Idle,
            finder_search_generation: 0,
            finder_search_task: None,
            focus_handle,
            account_task: None,
            model_task: None,
            settings_store: services.settings_store,
            settings_load_task: None,
            setup_save_task: None,
            setup_error: None,
            settings_window: None,
            settings_view: None,
            settings_subscription: None,
            subscriptions,
        };
        main.load_history(window, cx);
        let weak = cx.weak_entity();
        window.on_window_should_close(cx, move |window, cx| {
            weak.update(cx, |main, cx| main.request_close(window, cx))
                .unwrap_or(true)
        });
        main.subscriptions
            .push(cx.on_app_quit(Self::prepare_shutdown));
        main.restore_account(window, cx);
        main.load_settings(window, cx);
        main
    }

    #[allow(clippy::too_many_lines)]
    fn subscribe_to_children(
        composer: &Entity<PromptComposer>,
        sidebar: &Entity<SidebarView>,
        conversation: &Entity<ConversationView>,
        workbench: Option<&Entity<AgentWorkbench>>,
        finder_input: &Entity<InputState>,
        window: &Window,
        cx: &mut Context<'_, Self>,
    ) -> Vec<Subscription> {
        let mut subscriptions = vec![
            cx.subscribe_in(
                composer,
                window,
                |main, _, event: &PromptComposerEvent, window, cx| match event {
                    PromptComposerEvent::Submit(request) => {
                        if let Some(target) = main.retry_target.take() {
                            main.retry_response(
                                target,
                                Some(request.generation.clone()),
                                window,
                                cx,
                            );
                        } else {
                            main.submit(request, window, cx);
                        }
                    }
                    PromptComposerEvent::Cancel => main.stop_active_run(window, cx),
                    PromptComposerEvent::ModeChanged => cx.notify(),
                    PromptComposerEvent::WorkspaceSelected(root) => {
                        main.register_project(root.clone(), window, cx);
                        main.sync_settings_setup(cx);
                    }
                    PromptComposerEvent::OpenWorkspacePanel(panel) => {
                        let section = match panel {
                            PromptWorkspacePanel::Files => WorkbenchSection::Files,
                            PromptWorkspacePanel::Changes => WorkbenchSection::Changes,
                        };

                        if let Some(workbench) = &main.workbench {
                            main.workbench_open = true;
                            workbench.update(cx, |workbench, cx| {
                                workbench.show_section(section, window, cx);
                                workbench.prepare_to_show(window, cx);
                            });
                            cx.notify();
                        }
                    }
                },
            ),
            cx.subscribe_in(
                sidebar,
                window,
                |main, _, event: &SidebarEvent, window, cx| match event {
                    SidebarEvent::NewChat => {
                        tracing::info!(operation = "sidebar.new_chat", "started a new chat");
                        main.navigate(None, window, cx);
                        main.composer
                            .update(cx, |composer, cx| composer.focus(window, cx));
                    }
                    SidebarEvent::OpenConversation(id) => {
                        tracing::info!(
                            conversation_id = id.0,
                            operation = "sidebar.open_conversation",
                            "selected a conversation"
                        );
                        main.navigate(Some(*id), window, cx);
                    }
                    SidebarEvent::RenameConversation(id, title) => {
                        main.rename_conversation(*id, title.clone(), window, cx);
                    }
                    SidebarEvent::SetPinned(id, pinned) => {
                        main.set_pinned(*id, *pinned, window, cx);
                    }
                    SidebarEvent::DeleteConversation(id) => {
                        main.confirm_delete_conversation(*id, window, cx);
                    }
                    SidebarEvent::RetryHistory => main.load_history(window, cx),
                    SidebarEvent::AddProject => main.choose_project(window, cx),
                    SidebarEvent::ActivateProject(project) => {
                        main.activate_project(project.clone(), window, cx);
                    }
                    SidebarEvent::ForgetProject(root) => {
                        main.forget_project(root.clone(), window, cx);
                    }
                    SidebarEvent::OpenSettings => {
                        tracing::info!(operation = "sidebar.open_settings", "settings requested");
                        main.open_settings(window, cx);
                    }
                    SidebarEvent::OpenSetup => {
                        tracing::info!(operation = "sidebar.open_setup", "setup requested");
                        main.open_setup(window, cx);
                    }
                    SidebarEvent::BeginLogin => {
                        tracing::info!(operation = "sidebar.begin_login", "login requested");
                        main.begin_login(window, cx);
                    }
                    SidebarEvent::SignOut => {
                        tracing::info!(operation = "sidebar.sign_out", "sign out requested");
                        main.sign_out(window, cx);
                    }
                    SidebarEvent::ToggleTheme => {
                        if let Err(error) = crate::theme::toggle(cx) {
                            tracing::error!(?error, "could not toggle the application theme");
                        }
                        cx.notify();
                    }
                },
            ),
            Self::subscribe_to_conversation(conversation, window, cx),
            cx.subscribe_in(
                finder_input,
                window,
                |main, _, event: &InputEvent, window, cx| {
                    if matches!(event, InputEvent::Change) {
                        main.finder_selected = 0;
                        main.schedule_finder_search(window, cx);
                    }
                },
            ),
        ];
        if let Some(workbench) = workbench {
            subscriptions.push(cx.subscribe_in(
                workbench,
                window,
                |main, _, event: &AgentWorkbenchEvent, _, cx| match event {
                    AgentWorkbenchEvent::Closed => {
                        main.workbench_open = false;
                        cx.notify();
                    }
                },
            ));
        }
        subscriptions
    }

    fn subscribe_to_conversation(
        conversation: &Entity<ConversationView>,
        window: &Window,
        cx: &mut Context<'_, Self>,
    ) -> Subscription {
        cx.subscribe_in(
            conversation,
            window,
            |main, _, event: &ConversationViewEvent, window, cx| match event {
                ConversationViewEvent::GenerationStarted => {
                    main.composer
                        .update(cx, |composer, cx| composer.set_generating(true, cx));
                }
                ConversationViewEvent::GenerationFinished(message) => {
                    main.save_response(message.clone(), window, cx);
                }
                ConversationViewEvent::StopGeneration(message_id) => {
                    main.stop_run(*message_id, window, cx);
                }
                ConversationViewEvent::AgentApproval(message_id, decision) => {
                    main.decide_agent_approval(*message_id, *decision, cx);
                }
                ConversationViewEvent::ApplyWorkspaceReview(message_id) => {
                    main.apply_workspace_review(*message_id, window, cx);
                }
                ConversationViewEvent::DiscardWorkspaceReview(message_id) => {
                    main.discard_workspace_review(*message_id, window, cx);
                }
                ConversationViewEvent::LoadEarlier => main.load_earlier(window, cx),
                ConversationViewEvent::LoadNewer => main.load_newer(window, cx),
                ConversationViewEvent::ReturnToLatest => {
                    if let Some(id) = main.active_conversation {
                        main.navigate(Some(id), window, cx);
                    }
                }
                ConversationViewEvent::Regenerate(message_id) => {
                    main.regenerate(*message_id, window, cx);
                }
                ConversationViewEvent::Retry(message_id) => {
                    main.retry_response(*message_id, None, window, cx);
                }
                ConversationViewEvent::PrepareContinue(message_id) => {
                    main.prepare_continuation(*message_id, window, cx);
                }
                ConversationViewEvent::ChooseModelForRetry(message_id) => {
                    main.retry_target = Some(*message_id);
                    main.composer.update(cx, |composer, cx| {
                        composer.prepare_model_retry(*message_id, window, cx);
                    });
                }
                ConversationViewEvent::FocusComposer => {
                    main.composer
                        .update(cx, |composer, cx| composer.focus(window, cx));
                }
                ConversationViewEvent::OpenProviderSettings => main.open_settings(window, cx),
                ConversationViewEvent::WorkspaceChange(change) => {
                    if let Some(workbench) = &main.workbench {
                        main.workbench_open = true;
                        workbench.update(cx, |workbench, cx| {
                            workbench.prepare_to_show(window, cx);
                            workbench.show_change(change.clone(), window, cx);
                        });
                        cx.notify();
                    }
                }
                ConversationViewEvent::WorkspaceInvalidated => {
                    if let Some(workbench) = &main.workbench {
                        let visible = main.workbench_open;
                        workbench.update(cx, |workbench, cx| {
                            workbench.mark_workspace_invalidated(visible, window, cx);
                        });
                    }
                }
            },
        )
    }
}
