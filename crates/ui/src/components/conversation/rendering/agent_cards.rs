use super::super::*;
use super::agent_activity::build_activity_rows;

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

pub(super) fn command_section_is_active(
    message: &Message,
    live_commands: &std::collections::HashMap<(MessageId, String), LiveCommand>,
) -> bool {
    message
        .agent_activities
        .iter()
        .filter(|activity| {
            activity.kind == AgentActivityKind::ToolCall && activity.tool_name == "run_command"
        })
        .any(|activity| {
            let has_result = message.agent_activities.iter().any(|candidate| {
                candidate.call_id == activity.call_id
                    && candidate.kind == AgentActivityKind::ToolResult
            });
            !has_result
                && live_commands
                    .get(&(message.id, activity.call_id.clone()))
                    .is_none_or(|command| command.result.is_none())
        })
}

pub(super) fn tool_call_section_is_active(message: &Message) -> bool {
    build_activity_rows(message)
        .iter()
        .any(|row| row.status.is_active())
}

pub(super) fn render_command_card(
    command: &WorkspaceCommand,
    status: &'static str,
    status_color: gpui_kit::Hsla,
    status_icon: Icon,
    output: String,
    cx: &App,
) -> AnyElement {
    v_flex()
        .w_full()
        .gap(px(8.))
        .p(px(10.))
        .rounded(cx.theme().radius_lg)
        .border_1()
        .border_color(status_color.opacity(0.38))
        .bg(cx.theme().secondary.opacity(0.28))
        .child(
            h_flex()
                .items_center()
                .gap(px(8.))
                .child(
                    Icon::empty()
                        .path("icons/agent-terminal.svg")
                        .small()
                        .text_color(cx.theme().muted_foreground),
                )
                .child(
                    v_flex()
                        .min_w_0()
                        .flex_1()
                        .gap(px(2.))
                        .child(div().font_medium().text_size(px(12.)).child("Command"))
                        .child(
                            div()
                                .w_full()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .font_family(cx.theme().mono_font_family.clone())
                                .text_size(cx.theme().mono_font_size)
                                .text_color(cx.theme().foreground)
                                .child(command.display()),
                        ),
                )
                .child(status_icon)
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(status_color)
                        .child(status),
                ),
        )
        .child(
            h_flex()
                .gap(px(12.))
                .text_size(px(10.))
                .text_color(cx.theme().muted_foreground)
                .child(format!("Working directory · {}", command.cwd))
                .child("Network · off")
                .child(format!("Timeout · {}s", command.timeout_seconds)),
        )
        .when(!output.is_empty(), |this| {
            this.child(
                v_flex()
                    .w_full()
                    .gap(px(4.))
                    .child(
                        div()
                            .text_size(px(10.))
                            .text_color(cx.theme().muted_foreground)
                            .child("Output"),
                    )
                    .child(
                        div()
                            .w_full()
                            .max_h(px(220.))
                            .overflow_y_scrollbar()
                            .p(px(8.))
                            .rounded(cx.theme().radius)
                            .bg(cx.theme().background.opacity(0.65))
                            .font_family(cx.theme().mono_font_family.clone())
                            .text_size(cx.theme().mono_font_size)
                            .child(output),
                    ),
            )
        })
        .into_any_element()
}

pub(super) fn command_status_icon(status: &str, cx: &App) -> Icon {
    match status {
        "Completed" => Icon::new(IconName::CircleCheck)
            .xsmall()
            .text_color(cx.theme().success),
        "Running" => Icon::new(IconName::LoaderCircle)
            .xsmall()
            .text_color(cx.theme().warning),
        "Awaiting approval" => Icon::empty()
            .path("icons/agent-shield-check.svg")
            .xsmall()
            .text_color(cx.theme().warning),
        "Rejected" | "Cancelled" => Icon::new(IconName::CircleX)
            .xsmall()
            .text_color(cx.theme().muted_foreground),
        _ => Icon::new(IconName::CircleX)
            .xsmall()
            .text_color(cx.theme().danger),
    }
}
