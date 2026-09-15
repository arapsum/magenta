use super::super::*;

pub(super) fn render_approval_actions(
    message_id: MessageId,
    approve_label: &str,
    can_approve_for_run: bool,
    view: &Entity<ConversationView>,
) -> AnyElement {
    let reject_view = view.clone();
    let approve_view = view.clone();
    let run_approval_view = view.clone();
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
                .small()
                .label(approve_label)
                .when(can_approve_for_run, Button::outline)
                .when(!can_approve_for_run, Button::primary)
                .tooltip("Allow this operation once")
                .on_click(move |_, _, cx| {
                    approve_view.update(cx, |view, cx| {
                        view.decide_agent_approval(magenta_core::AgentApprovalDecision::Approve, cx);
                    });
                }),
        )
        .when(can_approve_for_run, |this| {
            this.child(
                Button::new(("agent-approve-run", message_id.0))
                    .primary()
                    .small()
                    .label("Allow edits for this run")
                    .tooltip(
                        "Allow file edits for this agent run only. Commands and protected files still require approval.",
                    )
                    .on_click(move |_, _, cx| {
                        run_approval_view.update(cx, |view, cx| {
                            view.decide_agent_approval(
                                magenta_core::AgentApprovalDecision::ApproveWorkspaceEditsForRun,
                                cx,
                            );
                        });
                    }),
            )
        })
        .into_any_element()
}
