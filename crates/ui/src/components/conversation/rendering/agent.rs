mod activity;

use super::super::*;

use super::agent_cards::render_approval_actions;

#[cfg(test)]
#[path = "../../../../test/components/conversation/rendering/agent.rs"]
mod tests;

impl ConversationView {
    fn trace_is_active(message: &Message) -> bool {
        message.status == MessageStatus::Streaming
    }

    fn trace_is_final_answer_started(&self, message_id: MessageId) -> bool {
        self.streaming_message == Some(message_id)
            && self
                .generation_progress
                .as_ref()
                .is_some_and(|progress| progress.final_answer_started)
    }

    fn trace_is_open(&self, message: &Message) -> bool {
        self.trace_disclosure_overrides
            .get(&message.id)
            .map_or_else(
                || {
                    Self::trace_is_active(message)
                        && !self.trace_is_final_answer_started(message.id)
                },
                |override_state| *override_state == TraceDisclosureOverride::Open,
            )
    }

    fn set_trace_disclosure_override(
        &mut self,
        message_id: MessageId,
        open: bool,
        cx: &mut Context<'_, Self>,
    ) {
        self.trace_disclosure_overrides.insert(
            message_id,
            if open {
                TraceDisclosureOverride::Open
            } else {
                TraceDisclosureOverride::Closed
            },
        );
        cx.notify();
    }

    fn trace_elapsed(&self, message: &Message) -> Option<Duration> {
        message
            .assistant_trace
            .thinking_duration_ms
            .map(Duration::from_millis)
            .or_else(|| {
                self.generation_progress
                    .as_ref()
                    .filter(|progress| progress.message_id == message.id)
                    .map(GenerationProgress::elapsed)
            })
    }

    pub(super) fn render_assistant_trace(
        &self,
        message: &Message,
        cx: &App,
        view: &Entity<Self>,
    ) -> AnyElement {
        activity::render(self, message, cx, view)
    }

    fn render_trace_entry_detail(
        message_id: MessageId,
        entry: &AssistantTraceEntry,
        cx: &App,
    ) -> AnyElement {
        if entry.kind == AssistantTraceKind::ReasoningSummary {
            return render_reasoning_summary(message_id, entry, cx);
        }

        v_flex()
            .w_full()
            .gap_2()
            .when(trace_detail_is_meaningful(&entry.input), |this| {
                this.child(trace_detail_block("Input", &entry.input, cx))
            })
            .when(trace_detail_is_meaningful(&entry.output), |this| {
                this.child(trace_detail_block("Output", &entry.output, cx))
            })
            .when(
                !trace_detail_is_meaningful(&entry.input)
                    && !trace_detail_is_meaningful(&entry.output),
                |this| {
                    this.child(
                        div()
                            .text_size(rems(0.6875))
                            .text_color(cx.theme().muted_foreground)
                            .child("No details available yet."),
                    )
                },
            )
            .into_any_element()
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

const TRACE_VIEWPORT_MAX_HEIGHT: gpui_kit::Rems = rems(16.25);

fn trace_header_label(
    message: &Message,
    active: bool,
    elapsed: Option<Duration>,
    mode: &ConversationMode,
) -> String {
    match message.status {
        MessageStatus::Stopped => "Work stopped".to_owned(),
        MessageStatus::Failed => "Work failed".to_owned(),
        _ if active => trace_activity_summary(message).unwrap_or_else(|| {
            elapsed.map_or_else(
                || {
                    if *mode == ConversationMode::Chat {
                        "Thinking".to_owned()
                    } else {
                        "Working".to_owned()
                    }
                },
                |elapsed| {
                    if *mode == ConversationMode::Chat {
                        format!("Thinking for {}", format_elapsed(elapsed))
                    } else {
                        format!("Working for {}", format_elapsed(elapsed))
                    }
                },
            )
        }),
        _ => elapsed.map_or_else(
            || {
                trace_activity_summary(message).unwrap_or_else(|| {
                    if *mode == ConversationMode::Chat {
                        "Thought process".to_owned()
                    } else {
                        "Work details".to_owned()
                    }
                })
            },
            |elapsed| {
                if *mode == ConversationMode::Chat {
                    format!("Thought for {}", format_elapsed(elapsed))
                } else {
                    format!("Worked for {}", format_elapsed(elapsed))
                }
            },
        ),
    }
}

fn trace_activity_summary(message: &Message) -> Option<String> {
    let mut loaded = false;
    let mut read = false;
    let mut ran = false;
    let mut edited = false;
    let mut viewed = false;
    let mut delegated = false;
    let mut used_other = false;

    for entry in &message.assistant_trace.entries {
        if entry.kind != AssistantTraceKind::Tool {
            continue;
        }
        let name = entry.tool_name.as_deref().unwrap_or(&entry.title);
        match name {
            "load_skill" | "read_skill" | "tool_search" => loaded = true,
            "read_file" | "list_files" | "search_files" | "find_files" => read = true,
            "run_command" | "exec_command" | "shell" => ran = true,
            "apply_patch" | "create_file" | "edit_file" | "write_file" => edited = true,
            "view_image" | "read_image" => viewed = true,
            "spawn_agent" | "create_agent" => delegated = true,
            _ => used_other = true,
        }
    }

    let mut activities = Vec::new();
    if loaded {
        activities.push("loaded tools");
    }
    if read {
        activities.push("read files");
    }
    if ran {
        activities.push("ran commands");
    }
    if edited {
        activities.push("edited files");
    }
    if viewed {
        activities.push("viewed images");
    }
    if delegated {
        activities.push("created agents");
    }
    if activities.is_empty() && used_other {
        activities.push("used tools");
    }
    activities.truncate(3);

    let mut summary = activities.join(", ");
    let first = summary.get_mut(0..1)?;
    first.make_ascii_uppercase();
    Some(summary)
}

fn trace_entries_for_timeline(message: &Message) -> Vec<&AssistantTraceEntry> {
    let mut entries = message.assistant_trace.entries.iter().collect::<Vec<_>>();
    entries.sort_by_key(|entry| entry.sequence);
    entries
}

fn trace_entry_title(entry: &AssistantTraceEntry) -> String {
    if entry.kind == AssistantTraceKind::ReasoningSummary
        && entry.title.eq_ignore_ascii_case("thinking")
    {
        "Reasoning".to_owned()
    } else if entry.kind == AssistantTraceKind::Tool {
        humanize_tool_name(entry.tool_name.as_deref().unwrap_or(&entry.title))
    } else {
        entry.title.clone()
    }
}

fn trace_entry_activity_label(entry: &AssistantTraceEntry) -> String {
    if entry.kind == AssistantTraceKind::ReasoningSummary {
        return trace_entry_title(entry);
    }

    match entry.tool_name.as_deref().unwrap_or(&entry.title) {
        "load_skill" | "read_skill" => "Read skill".to_owned(),
        "tool_search" => "Loaded tools".to_owned(),
        "read_file" => "Read file".to_owned(),
        "list_files" | "find_files" => "Listed files".to_owned(),
        "search_files" | "search_text" | "search_code" => "Searched the project".to_owned(),
        "run_command" | "exec_command" | "shell" => "Ran command".to_owned(),
        "apply_patch" | "edit_file" | "write_file" => "Edited a file".to_owned(),
        "create_file" => "Created a file".to_owned(),
        "view_image" | "read_image" => "Viewed an image".to_owned(),
        "spawn_agent" | "create_agent" => "Created an agent".to_owned(),
        _ => trace_entry_title(entry),
    }
}

fn humanize_tool_name(value: &str) -> String {
    let mut words = value.split('_').filter(|word| !word.is_empty());
    let Some(first) = words.next() else {
        return "Tool".to_owned();
    };

    let mut label = first.to_owned();
    if let Some(initial) = label.get_mut(0..1) {
        initial.make_ascii_uppercase();
    }
    for word in words {
        label.push(' ');
        label.push_str(word);
    }
    label
}

fn trace_detail_is_meaningful(value: &str) -> bool {
    !matches!(value.trim(), "" | "{}" | "[]" | "null")
}

fn render_reasoning_summary(
    message_id: MessageId,
    entry: &AssistantTraceEntry,
    cx: &App,
) -> AnyElement {
    if entry.output.is_empty() {
        return div()
            .text_size(rems(0.6875))
            .text_color(cx.theme().muted_foreground)
            .child("No summary available yet.")
            .into_any_element();
    }

    TextView::markdown(
        format!("assistant-trace-summary-{}-{}", message_id.0, entry.key),
        entry.output.clone(),
    )
    .selectable(true)
    .style(conversation_text_style(cx))
    .text_size(rems(0.8125))
    .line_height(rems(1.25))
    .text_color(cx.theme().muted_foreground)
    .into_any_element()
}

const fn trace_status_label(status: AssistantTraceStatus) -> &'static str {
    match status {
        AssistantTraceStatus::Streaming => "Streaming",
        AssistantTraceStatus::Requested => "Requested",
        AssistantTraceStatus::Running => "Running",
        AssistantTraceStatus::AwaitingApproval => "Awaiting approval",
        AssistantTraceStatus::Completed => "Completed",
        AssistantTraceStatus::Rejected => "Rejected",
        AssistantTraceStatus::Failed => "Failed",
        AssistantTraceStatus::Stopped => "Stopped",
    }
}

fn trace_status_color(status: AssistantTraceStatus, cx: &App) -> gpui_kit::Hsla {
    match status {
        AssistantTraceStatus::Streaming | AssistantTraceStatus::Running => cx.theme().primary,
        AssistantTraceStatus::AwaitingApproval => cx.theme().warning,
        AssistantTraceStatus::Completed
        | AssistantTraceStatus::Rejected
        | AssistantTraceStatus::Stopped
        | AssistantTraceStatus::Requested => cx.theme().muted_foreground,
        AssistantTraceStatus::Failed => cx.theme().danger,
    }
}

fn trace_detail_block(label: &str, value: &str, cx: &App) -> AnyElement {
    v_flex()
        .w_full()
        .gap_1()
        .child(
            div()
                .text_size(rems(0.6875))
                .text_color(cx.theme().muted_foreground)
                .child(label.to_owned()),
        )
        .child(
            div()
                .w_full()
                .max_h(rems(10.))
                .overflow_y_scrollbar()
                .p_2()
                .rounded(cx.theme().radius)
                .bg(crate::components::visual::surface(
                    crate::components::visual::SurfaceLevel::Recessed,
                    cx,
                ))
                .border_1()
                .border_color(cx.theme().border.opacity(0.48))
                .font_family(cx.theme().mono_font_family.clone())
                .text_size(cx.theme().mono_font_size)
                .child(value.to_owned()),
        )
        .into_any_element()
}
