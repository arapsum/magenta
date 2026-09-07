mod account;
mod deletion;
mod finder;
mod history;
mod render;
mod settings_window;
#[cfg(test)]
mod tests;

use std::sync::Arc;

use gpui::{
    AnyElement, AppContext as _, Context, Entity, FocusHandle, Focusable as _, KeyBinding,
    MouseButton, Render, Role, SharedString, StatefulInteractiveElement as _, Subscription, Task,
    Window, WindowHandle, div, prelude::*, px,
};
use gpui_component::{
    ActiveTheme as _, Icon, IconName, Sizable as _, StyledExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputEvent, InputState},
    scroll::ScrollableElement as _,
    v_flex,
};
use magenta_application::{ConversationHistory, RegenerateMessage, SendMessage};
use magenta_core::{
    ConversationId, ModelCatalog, ProviderAccount, ProviderAuthenticator, SettingsStore,
};

use self::settings_window::{AccountSettingsState, SettingsWindow, SettingsWindowEvent};
use crate::components::{
    conversation::{ConversationView, ConversationViewEvent},
    prompt_input::{PromptComposer, PromptComposerEvent},
    sidebar::{SidebarEvent, SidebarView},
    titlebar, workspace,
};

#[derive(Clone, Debug, Default, Eq, PartialEq, gpui::Action)]
#[action(namespace = magenta)]
pub struct OpenConversationFinder;

#[derive(Clone, Debug, Default, Eq, PartialEq, gpui::Action)]
#[action(namespace = magenta)]
pub struct CloseConversationFinder;

#[derive(Clone, Debug, Default, Eq, PartialEq, gpui::Action)]
#[action(namespace = magenta)]
struct SelectNextFinderResult;

#[derive(Clone, Debug, Default, Eq, PartialEq, gpui::Action)]
#[action(namespace = magenta)]
struct SelectPreviousFinderResult;

#[derive(Clone, Debug, Default, Eq, PartialEq, gpui::Action)]
#[action(namespace = magenta)]
struct ConfirmFinderResult;

#[derive(Clone, Debug, Default, Eq, PartialEq, gpui::Action)]
#[action(namespace = magenta)]
struct ConfirmConversationDeletion;

#[derive(Clone, Debug, Default, Eq, PartialEq, gpui::Action)]
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
    send_message: SendMessage,
    regenerate_message: RegenerateMessage,
    authenticator: Arc<dyn ProviderAuthenticator>,
    model_catalog: Arc<dyn ModelCatalog>,
    history: ConversationHistory,
    storage_ready: StorageState,
    operation: history::Operation,
    operation_task: Option<Task<()>>,
    history_task: Option<Task<()>>,
    load_task: Option<Task<()>>,
    page_task: Option<Task<()>>,
    load_generation: u64,
    loading_conversation: Option<ConversationId>,
    deferred_navigation: Option<history::Navigation>,
    unsaved: Option<magenta_core::Message>,
    close_requested: CloseState,
    active_conversation: Option<ConversationId>,
    pending_deletion: Option<PendingDeletion>,
    account_state: AccountState,
    finder_open: PanelState,
    finder_input: Entity<InputState>,
    finder_selected: usize,
    focus_handle: FocusHandle,
    account_task: Option<Task<()>>,
    model_task: Option<Task<()>>,
    settings_store: Arc<dyn SettingsStore>,
    settings_load_task: Option<Task<()>>,
    settings_window: Option<WindowHandle<gpui_component::Root>>,
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
    pub settings_store: Arc<dyn SettingsStore>,
}

#[derive(Clone, Debug)]
enum AccountState {
    Restoring,
    SignedOut,
    WaitingForBrowser,
    Connected(ProviderAccount),
    Failed(String),
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
        let finder_input = cx.new(|cx| InputState::new(window, cx).placeholder("Search chats…"));
        let focus_handle = cx.focus_handle();
        focus_handle.focus(window, cx);
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
        composer.update(cx, |composer, cx| composer.set_storage_ready(false, cx));
        let subscriptions = Self::subscribe_to_children(
            &composer,
            &sidebar,
            &conversation,
            &finder_input,
            window,
            cx,
        );

        let mut main = Self {
            sidebar,
            composer,
            conversation,
            send_message,
            regenerate_message,
            authenticator: services.authenticator,
            model_catalog: services.model_catalog,
            history,
            storage_ready: StorageState::Loading,
            operation: history::Operation::Idle,
            operation_task: None,
            history_task: None,
            load_task: None,
            page_task: None,
            load_generation: 0,
            loading_conversation: None,
            deferred_navigation: None,
            unsaved: None,
            close_requested: CloseState::Open,
            active_conversation: None,
            pending_deletion: None,
            account_state: AccountState::Restoring,
            finder_open: PanelState::Closed,
            finder_input,
            finder_selected: 0,
            focus_handle,
            account_task: None,
            model_task: None,
            settings_store: services.settings_store,
            settings_load_task: None,
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

    fn subscribe_to_children(
        composer: &Entity<PromptComposer>,
        sidebar: &Entity<SidebarView>,
        conversation: &Entity<ConversationView>,
        finder_input: &Entity<InputState>,
        window: &Window,
        cx: &mut Context<'_, Self>,
    ) -> Vec<Subscription> {
        vec![
            cx.subscribe_in(
                composer,
                window,
                |main, _, event: &PromptComposerEvent, window, cx| match event {
                    PromptComposerEvent::Submit(request) => main.submit(request, window, cx),
                    PromptComposerEvent::Cancel => main.cancel_generation(cx),
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
                    SidebarEvent::OpenSettings => {
                        tracing::info!(operation = "sidebar.open_settings", "settings requested");
                        main.open_settings(window, cx);
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
                    ConversationViewEvent::LoadEarlier => main.load_earlier(window, cx),
                    ConversationViewEvent::Regenerate(message_id) => {
                        main.regenerate(*message_id, window, cx);
                    }
                },
            ),
            cx.subscribe_in(
                finder_input,
                window,
                |main, _, event: &InputEvent, _, cx| {
                    if matches!(event, InputEvent::Change) {
                        main.finder_selected = 0;
                        cx.notify();
                    }
                },
            ),
        ]
    }
}
