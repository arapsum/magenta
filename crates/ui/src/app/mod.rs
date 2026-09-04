use gpui::{Entity, Render, SharedString, Subscription, Window, div, prelude::*};
use gpui_component::ActiveTheme as _;

use crate::components::{
    prompt_input::{PromptComposer, PromptComposerEvent},
    sidebar, titlebar, workspace,
};

use sidebar::Destination;

pub struct MainView {
    text: SharedString,
    sidebar_collapsed: bool,
    active_destination: Destination,
    composer: Entity<PromptComposer>,
    _subscriptions: Vec<Subscription>,
}

impl MainView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let composer = cx.new(|cx| PromptComposer::new(window, cx));
        let _subscriptions = vec![cx.subscribe(
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
        )];

        Self {
            text: "Adleio".into(),
            sidebar_collapsed: false,
            active_destination: Destination::StartupStrategy,
            composer,
            _subscriptions,
        }
    }

    pub(crate) fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        self.sidebar_collapsed = !self.sidebar_collapsed;
        cx.notify();
    }

    pub(crate) fn select_sidebar_destination(
        &mut self,
        destination: Destination,
        cx: &mut Context<Self>,
    ) {
        if self.active_destination == destination {
            return;
        }

        self.active_destination = destination;
        cx.notify();
    }
}

impl Render for MainView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let sidebar = sidebar::render(self.sidebar_collapsed, self.active_destination, cx);
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
                    .child(sidebar)
                    .child(workspace::render(
                        self.text.clone(),
                        self.composer.clone(),
                        cx,
                    )),
            )
    }
}
