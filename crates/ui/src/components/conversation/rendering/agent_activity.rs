use std::collections::HashSet;

use super::super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ActivityStatus {
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

    pub(super) const fn is_active(self) -> bool {
        matches!(self, Self::Working | Self::AwaitingApproval)
    }
}

#[derive(Clone, Debug)]
pub(super) struct ActivityRow {
    pub(super) call_id: String,
    pub(super) tool_name: String,
    pub(super) title: String,
    pub(super) arguments: String,
    pub(super) output: Option<String>,
    pub(super) status: ActivityStatus,
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

pub(super) fn build_activity_rows(message: &Message) -> Vec<ActivityRow> {
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

pub(super) fn agent_tool_icon(tool_name: &str, cx: &App) -> Icon {
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

pub(super) fn render_activity_detail(row: &ActivityRow, cx: &App) -> AnyElement {
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
                .rounded(px(9.))
                .border_1()
                .border_color(cx.theme().foreground.opacity(0.05))
                .bg(crate::components::visual::surface(
                    crate::components::visual::SurfaceLevel::Recessed,
                    cx,
                ))
                .font_family(cx.theme().mono_font_family.clone())
                .text_size(px(11.))
                .text_color(cx.theme().foreground)
                .child(value.to_owned()),
        )
        .into_any_element()
}

pub(super) fn section_title(
    label: &'static str,
    count: usize,
    count_label: &str,
    cx: &App,
) -> AnyElement {
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

pub(super) fn activity_title_element(title: &str, status: ActivityStatus, cx: &App) -> AnyElement {
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
