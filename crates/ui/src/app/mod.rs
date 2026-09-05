use std::sync::Arc;

use gpui::{
    AnyElement, AppContext as _, Context, Entity, Render, SharedString, Subscription, Task, Window,
    div, prelude::*, px,
};
use gpui_component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _, StyledExt as _, WindowExt,
    button::{Button, ButtonVariants as _},
    h_flex, v_flex,
};
use magenta_application::{
    PendingGeneration, PendingRegeneration, RegenerateMessage, RegenerateMessageInput, SendMessage,
    SendMessageInput, SendTarget,
};
use magenta_core::{
    Attachment, ConversationId, ModelCatalog, ProviderAccount, ProviderAuthenticator,
};

use crate::components::{
    conversation::{ConversationView, ConversationViewEvent, DemoCatalog, DemoThread, summary_for},
    prompt_input::{PromptComposer, PromptComposerEvent, PromptRequest},
    sidebar::{SidebarEvent, SidebarView},
    titlebar, workspace,
};
use crate::{MagentaError, notification_for_error};

pub struct MainView {
    text: SharedString,
    sidebar: Entity<SidebarView>,
    composer: Entity<PromptComposer>,
    conversation: Entity<ConversationView>,
    send_message: SendMessage,
    regenerate_message: RegenerateMessage,
    authenticator: Arc<dyn ProviderAuthenticator>,
    model_catalog: Arc<dyn ModelCatalog>,
    catalog: DemoCatalog,
    active_conversation: Option<ConversationId>,
    account_state: AccountState,
    account_panel_open: bool,
    account_task: Option<Task<()>>,
    model_task: Option<Task<()>>,
    subscriptions: Vec<Subscription>,
}

#[derive(Clone, Debug)]
enum AccountState {
    Restoring,
    SignedOut,
    WaitingForBrowser,
    Connected(ProviderAccount),
    Failed(String),
}

impl MainView {
    pub fn new(
        send_message: SendMessage,
        regenerate_message: RegenerateMessage,
        authenticator: Arc<dyn ProviderAuthenticator>,
        model_catalog: Arc<dyn ModelCatalog>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        let composer = cx.new(|cx| PromptComposer::new(window, cx));
        let sidebar = cx.new(|cx| SidebarView::new(window, cx));
        let conversation = cx.new(|cx| ConversationView::new(composer.clone(), window, cx));
        let subscriptions = vec![
            cx.subscribe_in(
                &composer,
                window,
                |main, _, event: &PromptComposerEvent, window, cx| match event {
                    PromptComposerEvent::Submit(request) => main.submit(request, window, cx),
                    PromptComposerEvent::Cancel => main.cancel_generation(cx),
                },
            ),
            cx.subscribe_in(
                &sidebar,
                window,
                |main, _, event: &SidebarEvent, window, cx| match event {
                    SidebarEvent::NewChat => {
                        tracing::info!(operation = "sidebar.new_chat", "started a new chat");
                        main.show_new_chat(cx);
                        main.composer
                            .update(cx, |composer, cx| composer.focus(window, cx));
                    }
                    SidebarEvent::OpenConversation(id) => {
                        tracing::info!(
                            conversation_id = id.0,
                            operation = "sidebar.open_conversation",
                            "selected a conversation"
                        );
                        main.open_conversation(*id, cx);
                    }
                    SidebarEvent::OpenSettings => {
                        tracing::info!(operation = "sidebar.open_settings", "settings requested");
                        main.account_panel_open = !main.account_panel_open;
                        cx.notify();
                    }
                },
            ),
            cx.subscribe_in(
                &conversation,
                window,
                |main, _, event: &ConversationViewEvent, window, cx| match event {
                    ConversationViewEvent::GenerationStarted => {
                        main.composer
                            .update(cx, |composer, cx| composer.set_generating(true, cx));
                        main.sync_active_thread(cx);
                    }
                    ConversationViewEvent::GenerationFinished => {
                        main.composer
                            .update(cx, |composer, cx| composer.set_generating(false, cx));
                        main.sync_active_thread(cx);
                    }
                    ConversationViewEvent::Updated => main.sync_active_thread(cx),
                    ConversationViewEvent::Regenerate(message_id) => {
                        main.regenerate(*message_id, window, cx);
                    }
                },
            ),
        ];

        let mut main = Self {
            text: "Adleio".into(),
            sidebar,
            composer,
            conversation,
            send_message,
            regenerate_message,
            authenticator,
            model_catalog,
            catalog: DemoCatalog::new(),
            active_conversation: None,
            account_state: AccountState::Restoring,
            account_panel_open: false,
            account_task: None,
            model_task: None,
            subscriptions,
        };
        main.restore_account(window, cx);
        main
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
    }

    fn show_new_chat(&mut self, cx: &mut Context<'_, Self>) {
        self.sync_active_thread(cx);
        self.active_conversation = None;
        self.conversation.update(cx, |conversation, cx| {
            conversation.clear(cx);
        });
        self.composer
            .update(cx, |composer, cx| composer.set_generating(false, cx));
    }

    fn open_conversation(&mut self, id: ConversationId, cx: &mut Context<'_, Self>) {
        let Some(mut thread) = self.catalog.thread(id).cloned() else {
            tracing::warn!(conversation_id = id.0, "demo conversation was not found");
            return;
        };
        self.sync_active_thread(cx);
        self.active_conversation = Some(id);
        let selected_generation = self.composer.update(cx, |composer, cx| {
            composer.set_configuration(&thread.conversation.generation, cx);
            composer.selected_configuration()
        });
        if let Some(generation) = selected_generation {
            thread.conversation.generation = generation;
        }
        self.conversation.update(cx, |conversation, cx| {
            conversation.load(thread, cx);
        });
    }

    fn submit(&mut self, request: &PromptRequest, window: &mut Window, cx: &mut Context<'_, Self>) {
        if !matches!(self.account_state, AccountState::Connected(_)) {
            self.account_panel_open = true;
            cx.notify();
            return;
        }
        if self.conversation.read(cx).is_streaming() {
            return;
        }

        let is_new_conversation = self.active_conversation.is_none();
        let Some(input) = self.send_input(request, cx) else {
            return;
        };
        let pending = match self.send_message.execute(input) {
            Ok(pending) => pending,
            Err(source) => {
                tracing::error!(
                    error = ?source,
                    operation = "prompt.prepare",
                    "could not prepare prompt"
                );
                let error = MagentaError::SendMessage { source };
                window.push_notification(notification_for_error(&error), cx);
                return;
            }
        };
        let PendingGeneration {
            conversation,
            user_message,
            assistant_message,
            stream,
        } = pending;
        let provider_id = conversation.generation.provider.clone();

        if is_new_conversation {
            self.sidebar.update(cx, |sidebar, cx| {
                sidebar.add_conversation(summary_for(&conversation), cx);
            });
            self.active_conversation = Some(conversation.id);
            self.conversation.update(cx, |view, cx| {
                view.load(
                    DemoThread {
                        conversation: conversation.clone(),
                        messages: Vec::new(),
                    },
                    cx,
                );
            });
        } else {
            self.conversation.update(cx, |view, cx| {
                view.set_generation(conversation.generation.clone(), cx);
            });
        }

        self.composer
            .update(cx, |composer, cx| composer.clear_after_submit(window, cx));
        self.conversation.update(cx, |view, cx| {
            view.start_generation(
                user_message,
                assistant_message,
                provider_id,
                stream,
                window,
                cx,
            );
        });
        tracing::info!(
            provider = %request.generation.provider.0,
            model = %request.generation.model.0,
            effort = ?request.generation.effort,
            attachment_count = request.attachments.len(),
            prompt_length = request.prompt.len(),
            operation = "prompt.submit",
            "local conversation prompt accepted"
        );
        self.sync_active_thread(cx);
    }

    fn send_input(
        &mut self,
        request: &PromptRequest,
        cx: &Context<'_, Self>,
    ) -> Option<SendMessageInput> {
        let (target, history) = if let Some(id) = self.active_conversation {
            let Some(thread) = self.catalog.thread(id) else {
                tracing::warn!(
                    conversation_id = id.0,
                    "active demo conversation was not found"
                );
                return None;
            };
            let Some(snapshot) = self.conversation.read(cx).snapshot() else {
                tracing::warn!(
                    conversation_id = id.0,
                    "active conversation has no loaded state"
                );
                return None;
            };
            (
                SendTarget::Existing(thread.conversation.clone()),
                snapshot.messages,
            )
        } else {
            let conversation_id = self.catalog.reserve_conversation_id();
            (SendTarget::New { conversation_id }, Vec::new())
        };
        let attachments = request
            .attachments
            .iter()
            .map(|path| Attachment {
                name: path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("Attachment")
                    .to_owned(),
                path: path.clone(),
            })
            .collect();

        Some(SendMessageInput {
            target,
            history,
            ids: self.catalog.reserve_message_ids(),
            prompt: request.prompt.to_string(),
            attachments,
            generation: request.generation.clone(),
        })
    }

    fn cancel_generation(&self, cx: &mut Context<'_, Self>) {
        self.conversation.update(cx, |conversation, cx| {
            conversation.cancel(cx);
        });
        cx.notify();
    }

    fn regenerate(
        &mut self,
        message_id: magenta_core::MessageId,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(conversation_id) = self.active_conversation else {
            return;
        };
        let Some(snapshot) = self.conversation.read(cx).snapshot() else {
            return;
        };
        if snapshot.conversation.id != conversation_id {
            return;
        }
        let generation = snapshot.conversation.generation.clone();
        let input = RegenerateMessageInput {
            conversation: snapshot.conversation,
            messages: snapshot.messages,
            target_message_id: message_id,
            assistant_message_id: self.catalog.reserve_message_id(),
        };
        let pending = match self.regenerate_message.execute(input) {
            Ok(pending) => pending,
            Err(source) => {
                tracing::error!(
                    error = ?source,
                    operation = "message.regenerate.prepare",
                    "could not prepare response regeneration"
                );
                let error = MagentaError::RegenerateMessage { source };
                window.push_notification(notification_for_error(&error), cx);
                return;
            }
        };
        let PendingRegeneration {
            target_message_id,
            assistant_message,
            provider_id,
            stream,
        } = pending;
        self.composer.update(cx, |composer, cx| {
            composer.set_configuration(&generation, cx);
        });
        self.conversation.update(cx, |view, cx| {
            view.regenerate(
                target_message_id,
                assistant_message,
                provider_id,
                stream,
                window,
                cx,
            );
        });
        self.sync_active_thread(cx);
    }

    fn sync_active_thread(&mut self, cx: &Context<'_, Self>) {
        if let Some(thread) = self.conversation.read(cx).snapshot() {
            self.catalog.replace_thread(thread);
        }
    }

    fn account_panel(&self, cx: &Context<'_, Self>) -> AnyElement {
        let view = cx.entity();
        let (status, detail) = self.account_status();
        let connected = matches!(self.account_state, AccountState::Connected(_));
        let waiting = matches!(
            self.account_state,
            AccountState::Restoring | AccountState::WaitingForBrowser
        );
        let action_view = view.clone();

        v_flex()
            .id("account-panel")
            .absolute()
            .top(px(18.))
            .right(px(22.))
            .w(px(320.))
            .gap(px(14.))
            .p(px(18.))
            .rounded(px(12.))
            .border_1()
            .border_color(cx.theme().border.opacity(0.9))
            .bg(cx.theme().popover)
            .shadow_lg()
            .child(Self::account_panel_header(action_view, cx))
            .child(
                div()
                    .font_medium()
                    .text_color(cx.theme().foreground)
                    .child(status),
            )
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(cx.theme().muted_foreground)
                    .child(detail),
            )
            .child(Self::account_panel_action(view, connected, waiting))
            .into_any_element()
    }

    fn account_status(&self) -> (&'static str, String) {
        match &self.account_state {
            AccountState::Restoring => (
                "Checking account",
                "Looking for a saved ChatGPT session.".to_owned(),
            ),
            AccountState::SignedOut => (
                "Not signed in",
                "Connect a ChatGPT account to load your OpenAI models.".to_owned(),
            ),
            AccountState::WaitingForBrowser => (
                "Finish sign-in in your browser",
                "Magenta is waiting for the local OAuth callback.".to_owned(),
            ),
            AccountState::Connected(account) => (
                "ChatGPT connected",
                account
                    .email
                    .clone()
                    .or_else(|| account.plan.clone())
                    .unwrap_or_else(|| "Your OpenAI account is ready.".to_owned()),
            ),
            AccountState::Failed(error) => ("Account unavailable", error.clone()),
        }
    }

    fn account_panel_header(view: Entity<Self>, cx: &Context<'_, Self>) -> impl IntoElement {
        h_flex()
            .items_center()
            .justify_between()
            .child(
                h_flex()
                    .items_center()
                    .gap(px(8.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(26.))
                            .rounded_full()
                            .bg(cx.theme().primary.opacity(0.16))
                            .text_color(cx.theme().primary)
                            .child(Icon::new(IconName::Bot).small()),
                    )
                    .child(div().font_medium().child("OpenAI account")),
            )
            .child(
                Button::new("close-account-panel")
                    .ghost()
                    .small()
                    .icon(IconName::Close)
                    .tooltip("Close account panel")
                    .on_click(move |_, _, cx| {
                        view.update(cx, |main, cx| {
                            main.account_panel_open = false;
                            cx.notify();
                        });
                    }),
            )
    }

    fn account_panel_action(view: Entity<Self>, connected: bool, waiting: bool) -> AnyElement {
        if connected {
            Button::new("sign-out")
                .secondary()
                .disabled(waiting)
                .label("Sign out")
                .on_click(move |_, window, cx| {
                    view.update(cx, |main, cx| main.sign_out(window, cx));
                })
                .into_any_element()
        } else {
            Button::new("sign-in-chatgpt")
                .primary()
                .disabled(waiting)
                .label(if waiting {
                    "Waiting…"
                } else {
                    "Sign in with ChatGPT"
                })
                .on_click(move |_, window, cx| {
                    view.update(cx, |main, cx| main.begin_login(window, cx));
                })
                .into_any_element()
        }
    }
}

impl Render for MainView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let _ = &self.subscriptions;
        let titlebar_title = self
            .active_conversation
            .and_then(|id| self.catalog.thread(id))
            .map_or_else(
                || "Magenta".to_owned(),
                |thread| thread.conversation.title.clone(),
            );
        let content: AnyElement = if self.active_conversation.is_some() {
            self.conversation.clone().into_any_element()
        } else {
            workspace::render(&self.text, self.composer.clone(), cx)
        };

        div()
            .relative()
            .flex()
            .flex_col()
            .size_full()
            .bg(cx.theme().tokens.background.background)
            .child(titlebar::render(titlebar_title))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .child(self.sidebar.clone())
                    .child(content),
            )
            .when(self.account_panel_open, |this| {
                this.child(self.account_panel(cx))
            })
    }
}
