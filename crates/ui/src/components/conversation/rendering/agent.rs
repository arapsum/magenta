use super::super::*;

impl ConversationView {
    pub(super) fn render_agent_activities(
        &self,
        message: &Message,
        cx: &App,
    ) -> Option<AnyElement> {
        if message.agent_activities.is_empty() {
            return None;
        }

        let commands = message
            .agent_activities
            .iter()
            .filter(|activity| {
                activity.kind == AgentActivityKind::ToolCall && activity.tool_name == "run_command"
            })
            .filter_map(|activity| self.render_command_activity(message, activity, cx));
        let activities = message
            .agent_activities
            .iter()
            .filter(|activity| activity.tool_name != "run_command")
            .map(|activity| {
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
                .gap(px(8.))
                .children(commands)
                .children(activities)
                .into_any_element(),
        )
    }

    fn render_command_activity(
        &self,
        message: &Message,
        activity: &AgentActivity,
        cx: &App,
    ) -> Option<AnyElement> {
        let live = self
            .live_commands
            .get(&(message.id, activity.call_id.clone()));
        let pending_command = self
            .pending_agent_approval
            .as_ref()
            .and_then(|(_, request)| {
                if request.tool_call_id != activity.call_id {
                    return None;
                }
                match &request.subject {
                    AgentApprovalSubject::Command(command) => Some(command.clone()),
                    AgentApprovalSubject::Workspace { .. } => None,
                }
            });
        let command = live
            .map(|live| live.command.clone())
            .or(pending_command)
            .or_else(|| serde_json::from_str::<WorkspaceCommand>(&activity.detail).ok())?;
        let result_activity = message.agent_activities.iter().rev().find(|candidate| {
            candidate.call_id == activity.call_id && candidate.kind == AgentActivityKind::ToolResult
        });
        let persisted_result = result_activity
            .and_then(|result| serde_json::from_str::<WorkspaceCommandResult>(&result.detail).ok());
        let result = live
            .and_then(|live| live.result.as_ref())
            .or(persisted_result.as_ref());
        let stdout = live.map_or_else(
            || result.map_or("", |result| result.stdout.as_str()),
            |live| live.stdout.as_str(),
        );
        let stderr = live.map_or_else(
            || result.map_or("", |result| result.stderr.as_str()),
            |live| live.stderr.as_str(),
        );
        let pending = self
            .pending_agent_approval
            .as_ref()
            .is_some_and(|(_, request)| request.tool_call_id == activity.call_id);
        let (status, status_color) = result.map_or_else(
            || {
                result_activity.map_or_else(
                    || {
                        if live.is_some() {
                            ("Running", cx.theme().warning)
                        } else if pending {
                            ("Awaiting approval", cx.theme().warning)
                        } else {
                            ("Requested", cx.theme().muted_foreground)
                        }
                    },
                    |result| {
                        if result.detail.contains("rejected") {
                            ("Rejected", cx.theme().muted_foreground)
                        } else {
                            ("Failed", cx.theme().danger)
                        }
                    },
                )
            },
            |result| match result.status {
                WorkspaceCommandStatus::Exited if result.exit_code == Some(0) => {
                    ("Completed", cx.theme().success)
                }
                WorkspaceCommandStatus::Exited => ("Exited", cx.theme().warning),
                WorkspaceCommandStatus::TimedOut => ("Timed out", cx.theme().danger),
                WorkspaceCommandStatus::Cancelled => ("Cancelled", cx.theme().muted_foreground),
                WorkspaceCommandStatus::Failed => ("Failed", cx.theme().danger),
            },
        );
        let mut output = match (stdout.is_empty(), stderr.is_empty()) {
            (false, false) => format!("{stdout}\n{stderr}"),
            (false, true) => stdout.to_owned(),
            (true, false) => stderr.to_owned(),
            (true, true) => String::new(),
        };
        if output.is_empty()
            && let Some(result) = result_activity
        {
            output.clone_from(&result.detail);
        }

        Some(render_command_card(
            &command,
            status,
            status_color,
            output,
            cx,
        ))
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
        let approval_reason = approval.reason.clone();
        let (approval_meta, detail, approve_label) = match &approval.subject {
            AgentApprovalSubject::Workspace { path, diff, .. } => (
                format!("{} · {path}", approval.tool_name),
                diff.clone(),
                "Approve",
            ),
            AgentApprovalSubject::Command(command) => (
                format!("Run in {} · network off", command.cwd),
                Some(command.display()),
                "Run",
            ),
        };
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
                        .child(approval_meta),
                )
                .child(div().text_size(px(12.)).child(approval_reason))
                .when_some(detail, |this, detail| {
                    this.child(
                        div()
                            .max_h(px(180.))
                            .overflow_y_scrollbar()
                            .p(px(8.))
                            .rounded(px(6.))
                            .bg(cx.theme().background.opacity(0.55))
                            .font_family(cx.theme().mono_font_family.clone())
                            .text_size(px(11.))
                            .child(detail),
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
                                .label(approve_label)
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

fn render_command_card(
    command: &WorkspaceCommand,
    status: &'static str,
    status_color: gpui::Hsla,
    output: String,
    cx: &App,
) -> AnyElement {
    v_flex()
        .w_full()
        .gap(px(8.))
        .p(px(10.))
        .rounded(px(8.))
        .border_1()
        .border_color(cx.theme().border.opacity(0.7))
        .bg(cx.theme().secondary.opacity(0.35))
        .child(
            h_flex()
                .items_center()
                .gap(px(6.))
                .child(Icon::empty().path("icons/code.svg").xsmall())
                .child(div().font_medium().text_size(px(12.)).child("Command"))
                .child(
                    div()
                        .ml_auto()
                        .text_size(px(11.))
                        .text_color(status_color)
                        .child(status),
                ),
        )
        .child(
            div()
                .w_full()
                .p(px(8.))
                .rounded(px(6.))
                .bg(cx.theme().background.opacity(0.65))
                .font_family(cx.theme().mono_font_family.clone())
                .text_size(cx.theme().mono_font_size)
                .child(command.display()),
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
                div()
                    .w_full()
                    .max_h(px(220.))
                    .overflow_y_scrollbar()
                    .p(px(8.))
                    .rounded(px(6.))
                    .bg(cx.theme().background.opacity(0.65))
                    .font_family(cx.theme().mono_font_family.clone())
                    .text_size(cx.theme().mono_font_size)
                    .child(output),
            )
        })
        .into_any_element()
}
