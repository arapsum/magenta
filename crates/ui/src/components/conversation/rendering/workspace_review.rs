use super::super::*;

impl ConversationView {
    pub(super) fn render_workspace_review(
        &self,
        message_id: MessageId,
        cx: &App,
        view: &Entity<Self>,
    ) -> Option<AnyElement> {
        let controller = self.agent_controller.as_ref()?;
        if self.streaming_message.is_some() || !controller.is_workspace_review_for(message_id) {
            return None;
        }

        let discard_view = view.clone();
        let apply_view = view.clone();

        Some(
            v_flex()
                .w_full()
                .overflow_hidden()
                .rounded(px(14.))
                .border_1()
                .border_color(cx.theme().border.opacity(0.68))
                .bg(crate::components::visual::surface(
                    crate::components::visual::SurfaceLevel::Raised,
                    cx,
                ))
                .child(
                    h_flex()
                        .p(px(13.))
                        .items_start()
                        .gap(px(9.))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_center()
                                .size(px(30.))
                                .rounded(px(8.))
                                .bg(cx.theme().accent.opacity(0.5))
                                .child(
                                    Icon::empty()
                                        .path("icons/agent-shield-check.svg")
                                        .small()
                                        .text_color(cx.theme().muted_foreground),
                                ),
                        )
                        .child(
                            v_flex()
                                .gap(px(3.))
                                .child(div().font_medium().child("Workspace changes are ready"))
                                .child(
                                    div()
                                        .text_size(px(12.))
                                        .text_color(cx.theme().muted_foreground)
                                        .child(
                                            "Review the workspace edits before applying them to the project.",
                                        ),
                                ),
                        ),
                )
                .child(
                    h_flex()
                        .p(px(10.))
                        .border_t_1()
                        .border_color(cx.theme().border.opacity(0.48))
                        .justify_end()
                        .gap(px(6.))
                        .child(
                            Button::new(("discard-agentfs", message_id.0))
                                .outline()
                                .small()
                                .label("Discard changes")
                                .on_click(move |_, window, cx| {
                                    discard_view.update(cx, |view, cx| {
                                        view.discard_workspace_review(window, cx);
                                    });
                                }),
                        )
                        .child(
                            Button::new(("apply-agentfs", message_id.0))
                                .primary()
                                .small()
                                .label("Apply to project")
                                .on_click(move |_, window, cx| {
                                    apply_view.update(cx, |view, cx| {
                                        view.apply_workspace_review(window, cx);
                                    });
                                }),
                        ),
                )
                .into_any_element(),
        )
    }
}
