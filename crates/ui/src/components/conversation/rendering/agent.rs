use super::super::*;

impl ConversationView {
    pub(super) fn render_agent_activities(message: &Message, cx: &App) -> Option<AnyElement> {
        if message.agent_activities.is_empty() {
            return None;
        }

        let activities = message.agent_activities.iter().map(|activity| {
            let (label, color) = match activity.kind {
                AgentActivityKind::ToolCall => ("Tool", cx.theme().muted_foreground),
                AgentActivityKind::ApprovalRequested => ("Permission", cx.theme().warning),
                AgentActivityKind::ToolResult => ("Result", cx.theme().muted_foreground),
            };
            h_flex()
                .w_full()
                .items_start()
                .gap(px(8.))
                .text_size(px(11.))
                .text_color(cx.theme().muted_foreground)
                .child(div().flex_none().text_color(color).child(label))
                .child(
                    div()
                        .min_w_0()
                        .flex_1()
                        .child(format!("{} · {}", activity.summary, activity.status)),
                )
                .into_any_element()
        });

        Some(
            v_flex()
                .w_full()
                .gap(px(4.))
                .p(px(9.))
                .rounded(px(8.))
                .border_1()
                .border_color(cx.theme().border.opacity(0.7))
                .bg(cx.theme().secondary.opacity(0.35))
                .children(activities)
                .into_any_element(),
        )
    }

    pub(super) fn render_agent_approval(
        &self,
        message_id: MessageId,
        cx: &App,
        view: &Entity<Self>,
    ) -> Option<AnyElement> {
        let (pending_message, approval) = self.pending_agent_approval.as_ref()?;
        if *pending_message != message_id {
            return None;
        }
        let approval_path = approval.path.clone();
        let approval_reason = approval.reason.clone();
        let diff = approval.diff.clone();
        let reject_view = view.clone();
        let approve_view = view.clone();

        Some(
            v_flex()
                .w_full()
                .gap(px(8.))
                .p(px(12.))
                .rounded(px(10.))
                .border_1()
                .border_color(cx.theme().warning.opacity(0.65))
                .bg(cx.theme().warning.opacity(0.08))
                .child(
                    div()
                        .font_medium()
                        .text_size(px(12.))
                        .child("Agent permission required"),
                )
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(cx.theme().muted_foreground)
                        .child(format!("{} · {approval_path}", approval.tool_name)),
                )
                .child(div().text_size(px(12.)).child(approval_reason))
                .when_some(diff, |this, diff| {
                    this.child(
                        div()
                            .max_h(px(180.))
                            .overflow_y_scrollbar()
                            .p(px(8.))
                            .rounded(px(6.))
                            .bg(cx.theme().background.opacity(0.55))
                            .font_family(cx.theme().mono_font_family.clone())
                            .text_size(px(11.))
                            .child(diff),
                    )
                })
                .child(
                    h_flex()
                        .justify_end()
                        .gap(px(6.))
                        .child(
                            Button::new(("agent-reject", message_id.0))
                                .outline()
                                .small()
                                .label("Reject")
                                .on_click(move |_, _, cx| {
                                    reject_view.update(cx, |view, cx| {
                                        view.decide_agent_approval(
                                            magenta_core::AgentApprovalDecision::Reject,
                                            cx,
                                        );
                                    });
                                }),
                        )
                        .child(
                            Button::new(("agent-approve", message_id.0))
                                .primary()
                                .small()
                                .label("Approve")
                                .on_click(move |_, _, cx| {
                                    approve_view.update(cx, |view, cx| {
                                        view.decide_agent_approval(
                                            magenta_core::AgentApprovalDecision::Approve,
                                            cx,
                                        );
                                    });
                                }),
                        ),
                )
                .into_any_element(),
        )
    }
}
