use gpui_kit::component::accordion::Accordion;

use super::super::*;

use super::agent_activity::{
    ActivityRow, activity_title_element, agent_tool_icon, build_activity_rows,
    render_activity_detail, section_title,
};
use super::agent_cards::{
    command_section_is_active, command_status_icon, render_approval_actions, render_command_card,
    tool_call_section_is_active,
};

#[cfg(test)]
#[path = "../../../../test/components/conversation/rendering/agent.rs"]
mod tests;

impl ConversationView {
    fn activity_section_is_active(&self, message: &Message, section: ActivitySection) -> bool {
        match section {
            ActivitySection::Commands => command_section_is_active(message, &self.live_commands),
            ActivitySection::ToolCalls => tool_call_section_is_active(message),
        }
    }

    fn activity_section_is_open(&self, message: &Message, section: ActivitySection) -> bool {
        self.activity_section_overrides
            .get(&(message.id, section))
            .map_or_else(
                || self.activity_section_is_active(message, section),
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
