use gpui::{AnyElement, Entity, Render, SharedString, Subscription, Window, div, prelude::*};
use gpui_component::{
    ActiveTheme as _, WindowExt,
    notification::{Notification, NotificationType},
};
use magenta_core::{Attachment, ConversationId, MessageRole, MessageStatus};

use crate::components::{
    conversation::{
        ConversationView, ConversationViewEvent, DemoCatalog, DemoThread, chat_model_for,
        fake_response, summary_for,
    },
    prompt_input::{PromptComposer, PromptComposerEvent, PromptRequest},
    sidebar::{SidebarEvent, SidebarView},
    titlebar, workspace,
};

pub struct MainView {
    text: SharedString,
    sidebar: Entity<SidebarView>,
    composer: Entity<PromptComposer>,
    conversation: Entity<ConversationView>,
    catalog: DemoCatalog,
    active_conversation: Option<ConversationId>,
    subscriptions: Vec<Subscription>,
}

impl MainView {
    pub fn new(window: &mut Window, cx: &mut Context<'_, Self>) -> Self {
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
        let model = chat_model_for(&thread.conversation);
        let effort = thread.conversation.effort;
        self.composer.update(cx, |composer, cx| {
            composer.set_configuration(model, effort, cx);
        });
        self.conversation.update(cx, |conversation, cx| {
            conversation.load(thread, cx);
        });
    }

    fn submit(&mut self, request: &PromptRequest, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.conversation.read(cx).is_streaming() {
            return;
        }

        let conversation = if let Some(id) = self.active_conversation {
            self.catalog
                .thread(id)
                .map(|thread| thread.conversation.clone())
        } else {
            let conversation = self.catalog.create_conversation(
                request.prompt.as_ref(),
                request.model,
                request.effort,
            );
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
            Some(conversation)
        };

        let Some(conversation) = conversation else {
            return;
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
        let user = self.catalog.new_message(
            conversation.id,
            MessageRole::User,
            request.prompt.to_string(),
            MessageStatus::Complete,
            attachments,
        );
        let assistant = self.catalog.new_message(
            conversation.id,
            MessageRole::Assistant,
            String::new(),
            MessageStatus::Streaming,
            Vec::new(),
        );
        let response = fake_response(request.prompt.as_ref());
        self.composer
            .update(cx, |composer, cx| composer.clear_after_submit(window, cx));
        self.conversation.update(cx, |view, cx| {
            view.start_generation(user, assistant, response, window, cx);
        });
        tracing::info!(
            model = ?request.model,
            effort = ?request.effort,
            attachment_count = request.attachments.len(),
            prompt_length = request.prompt.len(),
            operation = "prompt.submit",
            "local conversation prompt accepted"
        );
        self.sync_active_thread(cx);
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
        window: &Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(conversation_id) = self.active_conversation else {
            return;
        };
        let Some(prompt) = self.conversation.read(cx).regeneration_prompt(message_id) else {
            return;
        };
        let Some(thread) = self.catalog.thread(conversation_id) else {
            return;
        };
        let conversation = thread.conversation.clone();
        let assistant = self.catalog.new_message(
            conversation_id,
            MessageRole::Assistant,
            String::new(),
            MessageStatus::Streaming,
            Vec::new(),
        );
        self.composer.update(cx, |composer, cx| {
            composer.set_configuration(chat_model_for(&conversation), conversation.effort, cx);
        });
        self.conversation.update(cx, |view, cx| {
            view.regenerate(message_id, assistant, fake_response(&prompt), window, cx);
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
