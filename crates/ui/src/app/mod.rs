use gpui::{Entity, Render, SharedString, Subscription, Window, div, prelude::*};
use gpui_component::{
    ActiveTheme as _, WindowExt,
    notification::{Notification, NotificationType},
};

use crate::components::{
    prompt_input::{PromptComposer, PromptComposerEvent},
    sidebar::{SidebarEvent, SidebarView},
    titlebar, workspace,
};

pub struct MainView {
    text: SharedString,
    sidebar: Entity<SidebarView>,
    composer: Entity<PromptComposer>,
    subscriptions: Vec<Subscription>,
}

impl MainView {
    pub fn new(window: &mut Window, cx: &mut Context<'_, Self>) -> Self {
        let composer = cx.new(|cx| PromptComposer::new(window, cx));
        let sidebar = cx.new(|cx| SidebarView::new(window, cx));
        let subscriptions = vec![
            cx.subscribe(
                &composer,
                |_, _, event: &PromptComposerEvent, _cx| match event {
                    PromptComposerEvent::Submit(request) => {
                        tracing::info!(
                            model = ?request.model,
                            effort = ?request.effort,
                            attachment_count = request.attachments.len(),
                            prompt_length = request.prompt.len(),
                            operation = "prompt.submit",
                            "prompt accepted by the composer"
                        );
                    }
                },
            ),
            cx.subscribe_in(
                &sidebar,
                window,
                |main, _, event: &SidebarEvent, window, cx| match event {
                    SidebarEvent::NewChat => {
                        tracing::info!(operation = "sidebar.new_chat", "started a new chat");
                        main.composer
                            .update(cx, |composer, cx| composer.focus(window, cx));
                    }
                    SidebarEvent::OpenConversation(id) => {
                        tracing::info!(
                            conversation_id = id.0,
                            operation = "sidebar.open_conversation",
                            "selected a conversation"
                        );
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
        ];

        Self {
            text: "Adleio".into(),
            sidebar,
            composer,
            subscriptions,
        }
    }
}

impl Render for MainView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let _ = &self.subscriptions;
        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(cx.theme().tokens.background.background)
            .child(titlebar::render("Magenta"))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .child(self.sidebar.clone())
                    .child(workspace::render(&self.text, self.composer.clone(), cx)),
            )
    }
}
