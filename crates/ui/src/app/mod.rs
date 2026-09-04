use gpui::{AnyElement, Entity, Render, SharedString, Subscription, Window, div, prelude::*};
use gpui_component::{
    ActiveTheme as _, WindowExt,
    notification::{Notification, NotificationType},
};
use magenta_application::{
    PendingGeneration, PendingRegeneration, RegenerateMessage, RegenerateMessageInput, SendMessage,
    SendMessageInput, SendTarget,
};
use magenta_core::{Attachment, ConversationId};

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
    catalog: DemoCatalog,
    active_conversation: Option<ConversationId>,
    subscriptions: Vec<Subscription>,
}

impl MainView {
    pub fn new(
        send_message: SendMessage,
        regenerate_message: RegenerateMessage,
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
                        window.push_notification(
                            Notification::new()
                                .title("Settings are coming next")
                                .message("Theme selection is available from the sidebar today.")
                                .with_type(NotificationType::Info),
                            cx,
                        );
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

        Self {
            text: "Adleio".into(),
            sidebar,
            composer,
            conversation,
            send_message,
            regenerate_message,
            catalog: DemoCatalog::new(),
            active_conversation: None,
            subscriptions,
        }
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
        let Some(thread) = self.catalog.thread(id).cloned() else {
            tracing::warn!(conversation_id = id.0, "demo conversation was not found");
            return;
        };
        self.sync_active_thread(cx);
        self.active_conversation = Some(id);
        self.composer.update(cx, |composer, cx| {
            composer.set_configuration(&thread.conversation.generation, cx);
        });
        self.conversation.update(cx, |conversation, cx| {
            conversation.load(thread, cx);
        });
    }

    fn submit(&mut self, request: &PromptRequest, window: &mut Window, cx: &mut Context<'_, Self>) {
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
    }
}
