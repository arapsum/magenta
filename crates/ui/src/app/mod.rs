mod history;
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

    fn restore_account(&mut self, window: &Window, cx: &Context<'_, Self>) {
        let authenticator = Arc::clone(&self.authenticator);
        self.account_task = Some(cx.spawn_in(window, async move |view, window| {
            let result = authenticator.restore().await;
            _ = view.update_in(window, |main, window, cx| {
                main.account_task = None;
                match result {
                    Ok(Some(account)) => {
                        main.set_account_state(AccountState::Connected(account), cx);
                        main.load_models(window, cx);
                    }
                    Ok(None) => {
                        main.set_account_state(AccountState::SignedOut, cx);
                    }
                    Err(error) => {
                        tracing::warn!(
                            error = ?error,
                            operation = "account.restore",
                            "could not restore the OpenAI account"
                        );
                        main.set_account_state(AccountState::Failed(error.source.to_string()), cx);
                    }
                }
                cx.notify();
            });
        }));
    }

    fn load_models(&mut self, window: &Window, cx: &Context<'_, Self>) {
        self.model_task.take();
        let catalog = Arc::clone(&self.model_catalog);
        self.model_task = Some(cx.spawn_in(window, async move |view, window| {
            let result = catalog.models().await;
            _ = view.update_in(window, |main, _, cx| {
                main.model_task = None;
                match result {
                    Ok(models) => {
                        main.composer.update(cx, |composer, cx| {
                            composer.set_models(models, cx);
                        });
                    }
                    Err(error) => {
                        tracing::warn!(
                            error = ?error,
                            operation = "models.load",
                            "could not load OpenAI models"
                        );
                        main.set_account_state(AccountState::Failed(error.source.to_string()), cx);
                    }
                }
                cx.notify();
            });
        }));
    }

    fn begin_login(&mut self, window: &Window, cx: &mut Context<'_, Self>) {
        if self.account_task.is_some() {
            return;
        }

        self.set_account_state(AccountState::WaitingForBrowser, cx);
        let authenticator = Arc::clone(&self.authenticator);
        self.account_task = Some(cx.spawn_in(window, async move |view, window| {
            let session = match authenticator.begin_login().await {
                Ok(session) => session,
                Err(error) => {
                    _ = view.update_in(window, |main, _, cx| {
                        main.account_task = None;
                        main.set_account_state(AccountState::Failed(error.source.to_string()), cx);
                        cx.notify();
                    });
                    return;
                }
            };
            let url = session.authorization_url.clone();
            _ = view.update_in(window, |_, _, cx| {
                cx.open_url(&url);
            });
            let result = session.completion.await;
            _ = view.update_in(window, |main, window, cx| {
                main.account_task = None;
                match result {
                    Ok(account) => {
                        main.set_account_state(AccountState::Connected(account), cx);
                        main.load_models(window, cx);
                    }
                    Err(error) => {
                        main.set_account_state(AccountState::Failed(error.source.to_string()), cx);
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn sign_out(&mut self, window: &Window, cx: &mut Context<'_, Self>) {
        if self.account_task.is_some() {
            return;
        }

        self.model_task.take();
        let authenticator = Arc::clone(&self.authenticator);
        self.set_account_state(AccountState::SignedOut, cx);
        self.composer.update(cx, |composer, cx| {
            composer.set_models(Vec::new(), cx);
        });
        self.account_task = Some(cx.spawn_in(window, async move |view, window| {
            let result = authenticator.sign_out().await;
            _ = view.update_in(window, |main, _, cx| {
                main.account_task = None;
                if let Err(error) = result {
                    main.set_account_state(AccountState::Failed(error.source.to_string()), cx);
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn set_account_state(&mut self, state: AccountState, cx: &mut Context<'_, Self>) {
        let account = match &state {
            AccountState::Connected(account) => Some(account.clone()),
            AccountState::Restoring
            | AccountState::SignedOut
            | AccountState::WaitingForBrowser
            | AccountState::Failed(_) => None,
        };
        self.account_state = state;
        self.sidebar.update(cx, |sidebar, cx| {
            sidebar.set_account(account, cx);
        });
        if let Some(settings_view) = self.settings_view.as_ref() {
            let state = self.account_settings_state();
            settings_view.update(cx, |settings, cx| settings.set_account(state, cx));
        }
    }

    fn account_settings_state(&self) -> AccountSettingsState {
        match &self.account_state {
            AccountState::Restoring | AccountState::WaitingForBrowser => AccountSettingsState {
                waiting: true,
                ..Default::default()
            },
            AccountState::Connected(account) => AccountSettingsState {
                account: Some(account.clone()),
                ..Default::default()
            },
            AccountState::Failed(error) => AccountSettingsState {
                error: Some(error.clone()),
                ..Default::default()
            },
            AccountState::SignedOut => AccountSettingsState::default(),
        }
    }

    fn load_settings(&mut self, window: &Window, cx: &Context<'_, Self>) {
        let store = Arc::clone(&self.settings_store);
        self.settings_load_task = Some(cx.spawn_in(window, async move |view, window| {
            let result = store.load().await;
            _ = view.update_in(window, |main, _, cx| {
                main.settings_load_task = None;
                match result {
                    Ok(value) => crate::settings::replace(value, cx),
                    Err(error) => tracing::warn!(
                        error = %error.source,
                        operation = "settings.load",
                        "could not load settings; using defaults"
                    ),
                }
                cx.notify();
            });
        }));
    }

    fn open_settings(&mut self, window: &Window, cx: &mut Context<'_, Self>) {
        if let Some(handle) = self.settings_window
            && handle.is_active(cx).is_some()
        {
            _ = handle.update(cx, |_, settings_window, _| {
                settings_window.activate_window();
            });
            return;
        }

        let account = self.account_settings_state();
        match SettingsWindow::open(Arc::clone(&self.settings_store), account, cx) {
            Ok((handle, settings_view)) => {
                let subscription = cx.subscribe_in(
                    &settings_view,
                    window,
                    |main, _, event: &SettingsWindowEvent, window, cx| match event {
                        SettingsWindowEvent::BeginLogin => main.begin_login(window, cx),
                        SettingsWindowEvent::SignOut => main.sign_out(window, cx),
                        SettingsWindowEvent::TypographyChanged => {
                            main.conversation.update(cx, |conversation, cx| {
                                conversation.refresh_math_typography(cx);
                            });
                        }
                    },
                );
                self.settings_window = Some(handle);
                self.settings_view = Some(settings_view);
                self.settings_subscription = Some(subscription);
            }
            Err(error) => tracing::error!(
                ?error,
                operation = "settings.open",
                "could not open settings"
            ),
        }
    }

    fn move_finder_selection(&mut self, offset: isize, cx: &mut Context<'_, Self>) {
        if !self.finder_open.is_open() {
            return;
        }

        let query = self.finder_input.read(cx).value();
        let count = self
            .sidebar
            .read(cx)
            .matching_conversations(query.trim())
            .len()
            + 1;
        self.finder_selected = (self.finder_selected.cast_signed() + offset)
            .rem_euclid(count.cast_signed())
            .cast_unsigned();
        cx.notify();
    }

    fn confirm_finder_selection(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if !self.finder_open.is_open() {
            return;
        }

        let query = self.finder_input.read(cx).value();
        let matches = self.sidebar.read(cx).matching_conversations(query.trim());
        if let Some((id, _, _)) = matches.get(self.finder_selected) {
            self.select_finder_result(*id, window, cx);
        } else {
            self.new_chat_from_finder(window, cx);
        }
    }
    fn open_finder(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.finder_open = PanelState::Open;
        self.finder_selected = 0;
        self.finder_input.update(cx, |input, cx| {
            input.set_value("", window, cx);
        });
        self.finder_input
            .read(cx)
            .focus_handle(cx)
            .focus(window, cx);
        cx.notify();
    }

    fn close_finder(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if !self.finder_open.is_open() {
            return;
        }

        self.finder_open = PanelState::Closed;
        self.finder_selected = 0;
        self.finder_input.update(cx, |input, cx| {
            input.set_value("", window, cx);
        });
        if !self.sidebar.read(cx).is_collapsed() && window.viewport_size().width >= px(696.) {
            self.sidebar
                .update(cx, |sidebar, cx| sidebar.focus_finder_launcher(window, cx));
        } else {
            self.focus_handle.focus(window, cx);
        }
        cx.notify();
    }

    fn select_finder_result(
        &mut self,
        id: ConversationId,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.close_finder(window, cx);
        self.navigate(Some(id), window, cx);
    }

    fn new_chat_from_finder(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.close_finder(window, cx);
        self.navigate(None, window, cx);
        self.composer
            .update(cx, |composer, cx| composer.focus(window, cx));
    }

    fn finder_conversation_row(
        id: ConversationId,
        title: SharedString,
        updated: SharedString,
        selected: bool,
        view: Entity<Self>,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let accessibility_label = title.to_string();
        h_flex()
            .id(("finder-conversation", id.0))
            .debug_selector(move || format!("finder-conversation-{}", id.0))
            .role(Role::ListBoxOption)
            .aria_label(accessibility_label)
            .aria_selected(selected)
            .cursor_pointer()
            .w_full()
            .h(px(40.))
            .px(px(10.))
            .gap(px(12.))
            .rounded(px(8.))
            .when(selected, |this| {
                this.bg(cx.theme().accent)
                    .text_color(cx.theme().accent_foreground)
            })
            .hover(|this| this.bg(cx.theme().accent))
            .child(
                div()
                    .relative()
                    .flex_none()
                    .size(px(14.))
                    .text_color(cx.theme().muted_foreground)
                    .child(
                        div()
                            .absolute()
                            .top(px(1.))
                            .left(px(1.))
                            .size(px(11.))
                            .rounded(px(3.))
                            .border_1()
                            .border_color(cx.theme().muted_foreground),
                    )
                    .child(
                        div()
                            .absolute()
                            .bottom(px(0.))
                            .left(px(3.))
                            .size(px(3.))
                            .border_l_1()
                            .border_b_1()
                            .border_color(cx.theme().muted_foreground),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .font_medium()
                    .text_size(px(13.))
                    .child(title),
            )
            .child(
                div()
                    .flex_none()
                    .font_family(cx.theme().mono_font_family.clone())
                    .text_size(px(11.))
                    .text_color(cx.theme().muted_foreground.opacity(0.8))
                    .child(updated),
            )
            .on_click(move |_, window, cx| {
                view.update(cx, |main, cx| {
                    main.select_finder_result(id, window, cx);
                });
            })
            .into_any_element()
    }

    fn finder_new_chat_row(
        selected: bool,
        view: Entity<Self>,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        h_flex()
            .id("finder-new-chat")
            .role(Role::ListBoxOption)
            .aria_label("New chat")
            .aria_selected(selected)
            .cursor_pointer()
            .w_full()
            .h(px(40.))
            .px(px(10.))
            .gap(px(12.))
            .rounded(px(8.))
            .when(selected, |this| {
                this.bg(cx.theme().accent)
                    .text_color(cx.theme().accent_foreground)
            })
            .hover(|this| this.bg(cx.theme().accent))
            .child(
                Icon::new(IconName::Plus)
                    .xsmall()
                    .text_color(cx.theme().muted_foreground),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .font_medium()
                    .text_size(px(13.))
                    .child("New chat"),
            )
            .child(
                div()
                    .font_family(cx.theme().mono_font_family.clone())
                    .text_size(px(11.))
                    .text_color(cx.theme().muted_foreground.opacity(0.8))
                    .child(if cfg!(target_os = "macos") {
                        "⌘N"
                    } else {
                        "Ctrl N"
                    }),
            )
            .on_click(move |_, window, cx| {
                view.update(cx, |main, cx| {
                    main.new_chat_from_finder(window, cx);
                });
            })
            .into_any_element()
    }

    fn finder_results(
        &self,
        query: &str,
        matches: Vec<(ConversationId, SharedString, SharedString)>,
        view: Entity<Self>,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let has_matches = !matches.is_empty();
        let action_index = matches.len();
        let mut result_rows = v_flex().w_full();
        if has_matches {
            result_rows = result_rows.child(
                h_flex()
                    .h(px(28.))
                    .px(px(10.))
                    .font_semibold()
                    .text_size(px(10.))
                    .text_color(cx.theme().muted_foreground.opacity(0.8))
                    .child("CHATS"),
            );
        }
        for (index, (id, title, updated)) in matches.into_iter().enumerate() {
            result_rows = result_rows.child(Self::finder_conversation_row(
                id,
                title,
                updated,
                self.finder_selected == index,
                view.clone(),
                cx,
            ));
        }
        if !has_matches {
            result_rows = result_rows.child(
                div()
                    .w_full()
                    .px(px(10.))
                    .py(px(20.))
                    .text_center()
                    .text_size(px(13.))
                    .text_color(cx.theme().muted_foreground)
                    .child(if query.is_empty() {
                        "No conversations yet."
                    } else {
                        "No conversations match your search."
                    }),
            );
        }

        result_rows = result_rows.child(
            h_flex()
                .h(px(28.))
                .px(px(10.))
                .font_semibold()
                .text_size(px(10.))
                .text_color(cx.theme().muted_foreground.opacity(0.8))
                .child("ACTIONS"),
        );
        result_rows
            .child(Self::finder_new_chat_row(
                self.finder_selected == action_index,
                view,
                cx,
            ))
            .into_any_element()
    }

    fn finder_popover(&self, result_rows: AnyElement, cx: &Context<'_, Self>) -> AnyElement {
        let keycap = |label: &'static str| {
            h_flex()
                .h(px(18.))
                .min_w(px(24.))
                .justify_center()
                .px(px(5.))
                .rounded(px(5.))
                .border_1()
                .border_color(cx.theme().border)
                .bg(cx.theme().background)
                .font_family(cx.theme().mono_font_family.clone())
                .text_size(px(10.))
                .child(label)
        };
        v_flex()
            .id("conversation-finder-dialog")
            .key_context("ConversationFinder")
            .role(Role::Dialog)
            .aria_label("Search chats")
            .w(px(385.))
            .max_h(px(560.))
            .overflow_hidden()
            .rounded(px(14.))
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().popover)
            .shadow_lg()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div().p(px(4.)).child(
                    Input::new(&self.finder_input)
                        .appearance(false)
                        .bordered(false)
                        .h(px(36.))
                        .w_full()
                        .px(px(8.))
                        .rounded(px(10.))
                        .bg(cx.theme().muted.opacity(0.55))
                        .accessibility_id("conversation-finder-input")
                        .aria_label("Search chats")
                        .prefix(
                            Icon::new(IconName::Search)
                                .xsmall()
                                .text_color(cx.theme().muted_foreground),
                        ),
                ),
            )
            .child(
                div()
                    .id("finder-result-list")
                    .role(Role::ListBox)
                    .max_h(px(320.))
                    .overflow_y_scrollbar()
                    .p(px(4.))
                    .child(result_rows),
            )
            .child(
                h_flex()
                    .h(px(36.))
                    .px(px(14.))
                    .gap(px(14.))
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().muted.opacity(0.4))
                    .text_size(px(11.))
                    .text_color(cx.theme().muted_foreground)
                    .child(h_flex().gap(px(5.)).child(keycap("↑↓")).child("Navigate"))
                    .child(h_flex().gap(px(5.)).child(keycap("↵")).child("Open")),
            )
            .into_any_element()
    }

    fn finder_overlay(&self, cx: &Context<'_, Self>) -> AnyElement {
        let query = self.finder_input.read(cx).value().trim().to_owned();
        let matches = self.sidebar.read(cx).matching_conversations(&query);
        let view = cx.entity();
        let result_rows = self.finder_results(&query, matches, view.clone(), cx);
        let popover = self.finder_popover(result_rows, cx);

        div()
            .id("conversation-finder-overlay")
            .absolute()
            .inset_0()
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .bg(cx.theme().background.opacity(0.72))
                    .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                        view.update(cx, |main, cx| main.close_finder(window, cx));
                    }),
            )
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_start()
                    .justify_center()
                    .pt(px(86.))
                    .child(popover),
            )
            .into_any_element()
    }

    fn titlebar_controls(&self, cx: &Context<'_, Self>) -> AnyElement {
        let sidebar_view = self.sidebar.clone();
        let sidebar_collapsed = self.sidebar.read(cx).is_collapsed();
        let toggle_icon = if sidebar_collapsed {
            IconName::PanelLeftOpen
        } else {
            IconName::PanelLeftClose
        };
        let toggle_label = if sidebar_collapsed {
            "Show sidebar"
        } else {
            "Hide sidebar"
        };

        h_flex()
            .id("titlebar-controls")
            .h_full()
            .items_center()
            .gap(px(4.))
            .px(px(4.))
            .child(
                Button::new("titlebar-sidebar-toggle")
                    .ghost()
                    .small()
                    .icon(toggle_icon)
                    .tooltip(toggle_label)
                    .accessibility_id("toggle-sidebar")
                    .on_click(move |_, _, cx| {
                        sidebar_view.update(cx, SidebarView::toggle_collapsed);
                    }),
            )
            .child(
                self.composer
                    .read(cx)
                    .model_selector(self.composer.clone(), cx),
            )
            .into_any_element()
    }

    fn conversation_content(&self, cx: &Context<'_, Self>) -> AnyElement {
        if self.active_conversation.is_some() {
            self.conversation.clone().into_any_element()
        } else {
            workspace::render(self.composer.clone(), &self.sidebar, cx)
        }
    }

    fn main_panel(&self, content: AnyElement, narrow: bool, cx: &Context<'_, Self>) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .child(titlebar::render(self.titlebar_controls(cx)))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .when(narrow, gpui::Styled::p_0)
                    .when(!narrow, |this| this.p(px(12.)))
                    .child(
                        div()
                            .flex()
                            .size_full()
                            .min_h_0()
                            .min_w_0()
                            .overflow_hidden()
                            .rounded(px(16.))
                            .border_1()
                            .border_color(cx.theme().border)
                            .bg(cx.theme().tokens.background.background)
                            .when(narrow, |this| {
                                this.rounded(px(0.))
                                    .border_0()
                                    .border_color(cx.theme().transparent)
                            })
                            .child(content),
                    ),
            )
            .into_any_element()
    }

    fn render_frame(
        &self,
        content: AnyElement,
        narrow: bool,
        show_sidebar: bool,
        cx: &Context<'_, Self>,
    ) -> impl IntoElement {
        div()
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(|main, _: &OpenConversationFinder, window, cx| {
                main.open_finder(window, cx);
            }))
            .on_action(
                cx.listener(|main, _: &CloseConversationFinder, window, cx| {
                    main.close_finder(window, cx);
                }),
            )
            .on_action(cx.listener(|main, _: &SelectNextFinderResult, _, cx| {
                main.move_finder_selection(1, cx);
            }))
            .on_action(cx.listener(|main, _: &SelectPreviousFinderResult, _, cx| {
                main.move_finder_selection(-1, cx);
            }))
            .on_action(cx.listener(|main, _: &ConfirmFinderResult, window, cx| {
                main.confirm_finder_selection(window, cx);
            }))
            .on_action(cx.listener(|main, _: &titlebar::CloseWindow, window, cx| {
                if main.request_close(window, cx) {
                    window.remove_window();
                }
            }))
            .relative()
            .flex()
            .flex_row()
            .size_full()
            .bg(cx.theme().tokens.background.background)
            .when(show_sidebar, |this| this.child(self.sidebar.clone()))
            .child(self.main_panel(content, narrow, cx))
            .when(self.finder_open.is_open(), |this| {
                this.child(self.finder_overlay(cx))
            })
            .when(self.loading_conversation.is_some(), |this| {
                this.child(
                    div()
                        .absolute()
                        .top(px(36.))
                        .right(px(24.))
                        .p(px(8.))
                        .bg(cx.theme().background)
                        .child("Loading conversation…"),
                )
            })
            .when(
                self.unsaved.is_some() && self.operation == history::Operation::Idle,
                |this| {
                    this.child(
                        Button::new("retry-save")
                            .label("Response not saved · Retry")
                            .absolute()
                            .top(px(36.))
                            .right(px(24.))
                            .on_click(
                                cx.listener(|main, _, window, cx| main.retry_save(window, cx)),
                            ),
                    )
                },
            )
    }
}

impl Render for MainView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let _ = &self.subscriptions;
        let content = self.conversation_content(cx);
        let narrow = window.viewport_size().width < px(696.);
        let sidebar_collapsed = self.sidebar.read(cx).is_collapsed();
        let show_sidebar = !narrow && !sidebar_collapsed;
        self.render_frame(content, narrow, show_sidebar, cx)
    }
}
