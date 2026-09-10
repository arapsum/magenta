use std::collections::HashSet;

use gpui_kit::component::accordion::Accordion;

use super::super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ActivityStatus {
    Working,
    AwaitingApproval,
    Completed,
    Rejected,
    Failed,
}

impl ActivityStatus {
    const fn label(self) -> &'static str {
        match self {
            Self::Working => "Working",
            Self::AwaitingApproval => "Needs approval",
            Self::Completed => "Done",
            Self::Rejected => "Rejected",
            Self::Failed => "Failed",
        }
    }

    fn color(self, cx: &App) -> gpui_kit::Hsla {
        match self {
            Self::Working | Self::AwaitingApproval => cx.theme().warning,
            Self::Completed => cx.theme().success,
            Self::Rejected => cx.theme().muted_foreground,
            Self::Failed => cx.theme().danger,
        }
    }

    fn icon(self, cx: &App) -> Icon {
        let color = self.color(cx);
        match self {
            Self::Working => Icon::new(IconName::LoaderCircle).xsmall().text_color(color),
            Self::AwaitingApproval => Icon::empty()
                .path("icons/agent-shield-check.svg")
                .xsmall()
                .text_color(color),
            Self::Completed => Icon::new(IconName::CircleCheck).xsmall().text_color(color),
            Self::Rejected | Self::Failed => {
                Icon::new(IconName::CircleX).xsmall().text_color(color)
            }
        }
    }

    const fn is_active(self) -> bool {
        matches!(self, Self::Working | Self::AwaitingApproval)
    }
}

#[derive(Clone, Debug)]
struct ActivityRow {
    call_id: String,
    tool_name: String,
    title: String,
    arguments: String,
    output: Option<String>,
    status: ActivityStatus,
}

fn build_activity_row(message: &Message, activity: &AgentActivity) -> ActivityRow {
    let call = message
        .agent_activities
        .iter()
        .find(|candidate| {
            candidate.call_id == activity.call_id && candidate.kind == AgentActivityKind::ToolCall
        })
        .unwrap_or(activity);
    let approval = message.agent_activities.iter().rev().find(|candidate| {
        candidate.call_id == activity.call_id
            && candidate.kind == AgentActivityKind::ApprovalRequested
    });
    let result = message.agent_activities.iter().rev().find(|candidate| {
        candidate.call_id == activity.call_id && candidate.kind == AgentActivityKind::ToolResult
    });
    let status = activity_status(approval, result);
    let arguments = format_activity_detail(&call.detail);
    let output = result
        .filter(|result| !result.detail.trim().is_empty())
        .map(|result| format_activity_detail(&result.detail));

    ActivityRow {
        call_id: call.call_id.clone(),
        tool_name: call.tool_name.clone(),
        title: activity_title(&call.tool_name, &call.detail, status),
        arguments,
        output,
        status,
    }
}

fn build_activity_rows(message: &Message) -> Vec<ActivityRow> {
    let mut rows = Vec::new();
    let mut seen = HashSet::new();
    for activity in message
        .agent_activities
        .iter()
        .filter(|activity| activity.tool_name != "run_command")
    {
        if seen.insert(activity.call_id.clone()) {
            rows.push(build_activity_row(message, activity));
        }
    }
    rows
}

fn activity_status(
    approval: Option<&AgentActivity>,
    result: Option<&AgentActivity>,
) -> ActivityStatus {
    result.map_or_else(
        || {
            if approval.is_some() {
                ActivityStatus::AwaitingApproval
            } else {
                ActivityStatus::Working
            }
        },
        |result| {
            if result.status == "failed" {
                if result.detail.contains("rejected") {
                    ActivityStatus::Rejected
                } else {
                    ActivityStatus::Failed
                }
            } else {
                ActivityStatus::Completed
            }
        },
    )
}

fn activity_title(tool_name: &str, detail: &str, status: ActivityStatus) -> String {
    let arguments = serde_json::from_str::<serde_json::Value>(detail).ok();
    let path = arguments
        .as_ref()
        .and_then(|value| value.get("path"))
        .and_then(serde_json::Value::as_str)
        .map(display_path);

    match tool_name {
        "list_files" => format!(
            "Explored {}",
            path.unwrap_or_else(|| "workspace".to_owned())
        ),
        "search_text" => {
            let query = arguments
                .as_ref()
                .and_then(|value| value.get("query"))
                .and_then(serde_json::Value::as_str)
                .map_or_else(|| "workspace".to_owned(), compact_text);
            format!("Searched for {query}")
        }
        "read_file" => format!("Read {}", path.unwrap_or_else(|| "file".to_owned())),
        "create_file" => format!(
            "{} {}",
            if status == ActivityStatus::Completed {
                "Created"
            } else {
                "Create"
            },
            path.unwrap_or_else(|| "file".to_owned())
        ),
        "apply_patch" => format!(
            "{} {}",
            if status == ActivityStatus::Completed {
                "Updated"
            } else {
                "Update"
            },
            path.unwrap_or_else(|| "file".to_owned())
        ),
        _ => compact_text(&tool_name.replace('_', " ")),
    }
}

fn display_path(path: &str) -> String {
    if path.trim().is_empty() || path == "." {
        "workspace".to_owned()
    } else {
        compact_text(path)
    }
}

fn compact_text(value: &str) -> String {
    const MAX_CHARS: usize = 72;
    let compact = value.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut chars = compact.chars();
    let value = chars.by_ref().take(MAX_CHARS).collect::<String>();
    if chars.next().is_some() {
        format!("{value}…")
    } else {
        value
    }
}

fn format_activity_detail(value: &str) -> String {
    serde_json::from_str::<serde_json::Value>(value)
        .ok()
        .and_then(|value| serde_json::to_string_pretty(&value).ok())
        .unwrap_or_else(|| value.to_owned())
}

fn agent_tool_icon(tool_name: &str, cx: &App) -> Icon {
    let path = match tool_name {
        "list_files" => "icons/agent-list-check.svg",
        "search_text" | "read_file" => "icons/agent-file-search.svg",
        "create_file" => "icons/agent-file-plus.svg",
        "apply_patch" => "icons/agent-file-diff.svg",
        "run_command" => "icons/agent-terminal.svg",
        _ => "icons/agent-wrench.svg",
    };
    Icon::empty()
        .path(path)
        .small()
        .text_color(cx.theme().muted_foreground)
}

fn render_activity_detail(row: &ActivityRow, cx: &App) -> AnyElement {
    let mut detail = v_flex().w_full().gap(px(8.));
    if !row.arguments.is_empty() {
        detail = detail.child(activity_detail_block("Arguments", &row.arguments, cx));
    }
    if let Some(output) = row.output.as_deref() {
        detail = detail.child(activity_detail_block("Result", output, cx));
    }
    detail.into_any_element()
}

fn activity_detail_block(label: &'static str, value: &str, cx: &App) -> AnyElement {
    v_flex()
        .w_full()
        .gap(px(4.))
        .child(
            div()
                .text_size(px(10.))
                .text_color(cx.theme().muted_foreground)
                .child(label),
        )
        .child(
            div()
                .w_full()
                .max_h(px(180.))
                .overflow_y_scrollbar()
                .p(px(8.))
                .rounded(cx.theme().radius)
                .bg(cx.theme().background.opacity(0.65))
                .font_family(cx.theme().mono_font_family.clone())
                .text_size(px(11.))
                .text_color(cx.theme().foreground)
                .child(value.to_owned()),
        )
        .into_any_element()
}

fn section_title(label: &'static str, count: usize, count_label: &str, cx: &App) -> AnyElement {
    h_flex()
        .w_full()
        .min_w_0()
        .items_center()
        .gap(px(8.))
        .child(
            div()
                .min_w_0()
                .flex_1()
                .font_medium()
                .text_size(px(12.))
                .child(label),
        )
        .child(
            div()
                .flex_none()
                .text_size(px(11.))
                .text_color(cx.theme().muted_foreground)
                .child(format!("{count} {count_label}")),
        )
        .into_any_element()
}

fn activity_title_element(title: &str, status: ActivityStatus, cx: &App) -> AnyElement {
    h_flex()
        .w_full()
        .min_w_0()
        .items_center()
        .gap(px(8.))
        .child(
            div()
                .min_w_0()
                .flex_1()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .text_size(px(12.))
                .child(title.to_owned()),
        )
        .child(status.icon(cx))
        .child(
            div()
                .flex_none()
                .text_size(px(11.))
                .text_color(status.color(cx))
                .child(status.label()),
        )
        .into_any_element()
}

impl ConversationView {
    fn activity_section_is_active(message: &Message, section: ActivitySection) -> bool {
        match section {
            ActivitySection::Commands => command_section_is_active(message),
            ActivitySection::ToolCalls => tool_call_section_is_active(message),
        }
    }

    fn activity_section_is_open(&self, message: &Message, section: ActivitySection) -> bool {
        self.activity_section_overrides
            .get(&(message.id, section))
            .map_or_else(
                || Self::activity_section_is_active(message, section),
                |override_state| *override_state == ActivitySectionOverride::Open,
            )
    }

    fn set_activity_section_override(
        &mut self,
        message_id: MessageId,
        section: ActivitySection,
        open: bool,
        cx: &mut Context<'_, Self>,
    ) {
        self.activity_section_overrides.insert(
            (message_id, section),
            if open {
                ActivitySectionOverride::Open
            } else {
                ActivitySectionOverride::Closed
            },
        );
        cx.notify();
    }

    pub(super) fn render_agent_activities(
        &self,
        message: &Message,
        cx: &App,
        view: &Entity<Self>,
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
            .filter_map(|activity| self.render_command_activity(message, activity, cx))
            .collect::<Vec<_>>();
        let rows = build_activity_rows(message);
        let mut content = v_flex().w_full().gap(px(8.));
        if let Some(command_section) = self.render_command_section(message, commands, cx, view) {
            content = content.child(command_section);
        }
        if let Some(tool_call_section) = self.render_tool_call_section(message, rows, cx, view) {
            content = content.child(tool_call_section);
        }

        Some(content.into_any_element())
    }

    fn render_command_section(
        &self,
        message: &Message,
        commands: Vec<AnyElement>,
        cx: &App,
        view: &Entity<Self>,
    ) -> Option<AnyElement> {
        if commands.is_empty() {
            return None;
        }

        let message_id = message.id;
        let command_count = commands.len();
        let command_view = view.clone();
        let command_open = self.activity_section_is_open(message, ActivitySection::Commands);
        let command_label = if command_count == 1 {
            "command"
        } else {
            "commands"
        };
        let mut accordion = Accordion::new(("agent-commands", message_id.0))
            .multiple(false)
            .bordered(false)
            .small()
            .on_toggle_click(move |open_indices, _, cx| {
                command_view.update(cx, |view, cx| {
                    view.set_activity_section_override(
                        message_id,
                        ActivitySection::Commands,
                        open_indices.contains(&0),
                        cx,
                    );
                });
            });
        accordion = accordion.item(|item| {
            item.open(command_open)
                .icon(
                    Icon::empty()
                        .path("icons/agent-terminal.svg")
                        .small()
                        .text_color(cx.theme().muted_foreground),
                )
                .title(section_title("Commands", command_count, command_label, cx))
                .hover(|this| this.bg(cx.theme().accent.opacity(0.45)))
                .child(v_flex().w_full().gap(px(8.)).children(commands))
        });
        Some(accordion.into_any_element())
    }

    fn render_tool_call_section(
        &self,
        message: &Message,
        rows: Vec<ActivityRow>,
        cx: &App,
        view: &Entity<Self>,
    ) -> Option<AnyElement> {
        if rows.is_empty() {
            return None;
        }

        let message_id = message.id;
        let tool_call_count = rows.len();
        let activity_accordion = self.render_activity_accordion(message_id, rows, cx, view);
        let tool_view = view.clone();
        let tool_open = self.activity_section_is_open(message, ActivitySection::ToolCalls);
        let tool_label = if tool_call_count == 1 {
            "tool call"
        } else {
            "tool calls"
        };
        let mut accordion = Accordion::new(("agent-tool-calls", message_id.0))
            .multiple(false)
            .bordered(false)
            .small()
            .on_toggle_click(move |open_indices, _, cx| {
                tool_view.update(cx, |view, cx| {
                    view.set_activity_section_override(
                        message_id,
                        ActivitySection::ToolCalls,
                        open_indices.contains(&0),
                        cx,
                    );
                });
            });
        accordion = accordion.item(|item| {
            item.open(tool_open)
                .icon(
                    Icon::empty()
                        .path("icons/agent-wrench.svg")
                        .small()
                        .text_color(cx.theme().muted_foreground),
                )
                .title(section_title("Tool calls", tool_call_count, tool_label, cx))
                .hover(|this| this.bg(cx.theme().accent.opacity(0.45)))
                .child(activity_accordion)
        });
        Some(accordion.into_any_element())
    }

    fn render_activity_accordion(
        &self,
        message_id: MessageId,
        rows: Vec<ActivityRow>,
        cx: &App,
        view: &Entity<Self>,
    ) -> Accordion {
        let call_ids = rows
            .iter()
            .map(|row| row.call_id.clone())
            .collect::<Vec<_>>();
        let activity_view = view.clone();
        let mut accordion = Accordion::new(("agent-activity", message_id.0))
            .multiple(true)
            .bordered(false)
            .small()
            .on_toggle_click(move |open_indices, _, cx| {
                activity_view.update(cx, |view, cx| {
                    for (index, call_id) in call_ids.iter().enumerate() {
                        let key = (message_id, call_id.clone());
                        if open_indices.contains(&index) {
                            view.expanded_agent_activity_calls.insert(key);
                        } else {
                            view.expanded_agent_activity_calls.remove(&key);
                        }
                    }
                    cx.notify();
                });
            });

        for row in rows {
            let open = self
                .expanded_agent_activity_calls
                .contains(&(message_id, row.call_id.clone()));
            let status = row.status;
            let tool_name = row.tool_name.clone();
            let title = row.title.clone();
            let detail = render_activity_detail(&row, cx);
            accordion = accordion.item(|item| {
                item.open(open)
                    .icon(agent_tool_icon(&tool_name, cx))
                    .title(activity_title_element(&title, status, cx))
                    .hover(|this| this.bg(cx.theme().accent.opacity(0.45)))
                    .child(detail)
            });
        }
        accordion
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
            command_status_icon(status, cx),
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
                "Allow once",
            ),
            AgentApprovalSubject::Command(command) => (
                format!("Run in {} · network off", command.cwd),
                Some(command.display()),
                "Run",
            ),
        };
        let can_approve_for_run = approval.can_approve_for_run;

        Some(
            v_flex()
                .w_full()
                .gap(px(8.))
                .p(px(12.))
                .rounded(cx.theme().radius_lg)
                .border_1()
                .border_color(cx.theme().warning.opacity(0.65))
                .bg(cx.theme().warning.opacity(0.06))
                .child(
                    h_flex()
                        .items_center()
                        .gap(px(8.))
                        .child(
                            Icon::empty()
                                .path("icons/agent-shield-check.svg")
                                .small()
                                .text_color(cx.theme().warning),
                        )
                        .child(
                            div()
                                .font_medium()
                                .text_size(px(12.))
                                .child("Agent permission required"),
                        ),
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
                            .rounded(cx.theme().radius)
                            .bg(cx.theme().background.opacity(0.55))
                            .font_family(cx.theme().mono_font_family.clone())
                            .text_size(px(11.))
                            .child(detail),
                    )
                })
                .child(render_approval_actions(
                    message_id,
                    approve_label,
                    can_approve_for_run,
                    view,
                ))
                .into_any_element(),
        )
    }
}

fn render_approval_actions(
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

fn command_section_is_active(message: &Message) -> bool {
    message
        .agent_activities
        .iter()
        .filter(|activity| {
            activity.kind == AgentActivityKind::ToolCall && activity.tool_name == "run_command"
        })
        .any(|activity| {
            !message.agent_activities.iter().any(|candidate| {
                candidate.call_id == activity.call_id
                    && candidate.kind == AgentActivityKind::ToolResult
            })
        })
}

fn tool_call_section_is_active(message: &Message) -> bool {
    build_activity_rows(message)
        .iter()
        .any(|row| row.status.is_active())
}

fn render_command_card(
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

fn command_status_icon(status: &str, cx: &App) -> Icon {
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

#[cfg(test)]
mod tests {
    use magenta_core::ConversationId;

    use super::*;

    fn activity(
        kind: AgentActivityKind,
        call_id: &str,
        tool_name: &str,
        status: &str,
    ) -> AgentActivity {
        let detail = if kind == AgentActivityKind::ToolCall {
            match tool_name {
                "run_command" => {
                    r#"{"program":"cargo","args":[],"cwd":".","timeout_seconds":30}"#.to_owned()
                }
                _ => r#"{"path":"src/lib.rs"}"#.to_owned(),
            }
        } else {
            String::new()
        };
        AgentActivity {
            kind,
            call_id: call_id.to_owned(),
            tool_name: tool_name.to_owned(),
            status: status.to_owned(),
            summary: String::new(),
            detail,
        }
    }

    fn message(activities: Vec<AgentActivity>) -> Message {
        Message {
            id: MessageId::new(1),
            conversation_id: ConversationId::new(1),
            role: MessageRole::Assistant,
            content: String::new(),
            status: MessageStatus::Complete,
            attachments: Vec::new(),
            generation_outcome: None,
            agent_activities: activities,
        }
    }

    #[test]
    fn settled_sections_default_to_closed_and_new_activity_reopens_them() {
        let mut message = message(vec![
            activity(
                AgentActivityKind::ToolCall,
                "tool-1",
                "read_file",
                "requested",
            ),
            activity(
                AgentActivityKind::ToolResult,
                "tool-1",
                "read_file",
                "completed",
            ),
            activity(
                AgentActivityKind::ToolCall,
                "command-1",
                "run_command",
                "requested",
            ),
            activity(
                AgentActivityKind::ToolResult,
                "command-1",
                "run_command",
                "completed",
            ),
        ]);
        assert!(!tool_call_section_is_active(&message));
        assert!(!command_section_is_active(&message));

        message.agent_activities.push(activity(
            AgentActivityKind::ToolCall,
            "tool-2",
            "list_files",
            "requested",
        ));
        assert!(tool_call_section_is_active(&message));
        assert!(!command_section_is_active(&message));
    }

    #[test]
    fn commands_and_tool_calls_settle_independently() {
        let message = message(vec![
            activity(
                AgentActivityKind::ToolCall,
                "tool-1",
                "read_file",
                "requested",
            ),
            activity(
                AgentActivityKind::ToolResult,
                "tool-1",
                "read_file",
                "failed",
            ),
            activity(
                AgentActivityKind::ToolCall,
                "command-1",
                "run_command",
                "requested",
            ),
        ]);
        assert!(!tool_call_section_is_active(&message));
        assert!(command_section_is_active(&message));
    }
}
