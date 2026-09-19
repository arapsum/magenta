use gpui_kit::component::{accordion::Accordion, shimmer::ShimmerText, spinner::Spinner};

use super::super::*;

use super::agent_cards::render_approval_actions;

#[cfg(test)]
#[path = "../../../../test/components/conversation/rendering/agent.rs"]
mod tests;

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
                .gap(px(8.))
                .p(px(12.))
                .rounded(cx.theme().radius_lg)
                .border_1()
                .border_color(cx.theme().warning.opacity(0.55))
                .child(div().font_medium().child("Review staged workspace changes"))
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(cx.theme().muted_foreground)
                        .child(
                            "Changes are isolated in AgentFS and have not touched the project yet.",
                        ),
                )
                .child(
                    h_flex()
                        .justify_end()
                        .gap(px(6.))
                        .child(
                            Button::new(("discard-agentfs", message_id.0))
                                .outline()
                                .small()
                                .label("Discard")
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
                                .label("Apply changes")
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

    fn trace_is_active(message: &Message) -> bool {
        message.status == MessageStatus::Streaming
            || message.assistant_trace.entries.iter().any(|entry| {
                matches!(
                    entry.status,
                    AssistantTraceStatus::Streaming
                        | AssistantTraceStatus::Requested
                        | AssistantTraceStatus::Running
                        | AssistantTraceStatus::AwaitingApproval
                )
            })
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
        let message_id = message.id;
        let mut rows = v_flex().w_full().gap_1().flex_col_reverse().py_1().pr_1();

        for entry in trace_entries_for_timeline(message) {
            let default_open = entry.kind == AssistantTraceKind::ReasoningSummary;
            let open = self
                .trace_entry_overrides
                .get(&(message_id, entry.key.clone()))
                .copied()
                .unwrap_or(default_open);
            let entry_title = trace_entry_title(&entry);
            let title = Self::render_trace_entry_title(&entry, cx);
            let detail = Self::render_trace_entry_detail(message_id, &entry, cx);
            let entry_key = entry.key.clone();
            let entry_view = view.clone();
            let disclosure_label = if open {
                format!("Collapse {entry_title} details")
            } else {
                format!("Expand {entry_title} details")
            };
            let disclosure = Button::new(format!(
                "assistant-trace-entry-{}-{}",
                message_id.0, entry.key
            ))
            .ghost()
            .xsmall()
            .compact()
            .w_full()
            .toggled(open)
            .accessibility_label(disclosure_label)
            .child(
                h_flex()
                    .w_full()
                    .items_center()
                    .gap(px(7.))
                    .child(title)
                    .child(
                        Icon::new(if open {
                            IconName::ChevronUp
                        } else {
                            IconName::ChevronDown
                        })
                        .xsmall()
                        .text_color(cx.theme().muted_foreground),
                    ),
            )
            .on_click(move |_, _, cx| {
                entry_view.update(cx, |view, cx| {
                    view.trace_entry_overrides
                        .insert((message_id, entry_key.clone()), !open);
                    cx.notify();
                });
            });
            let row = v_flex()
                .w_full()
                .flex_none()
                .pl_3()
                .border_l_1()
                .border_color(trace_status_color(entry.status, cx).opacity(0.34))
                .child(disclosure)
                .when(open, |this| {
                    this.child(div().w_full().pl_2().pr_2().pb_2().child(detail))
                });
            rows = rows.child(row);
        }

        let trace_viewport = div()
            .id(("assistant-trace-viewport", message_id.0))
            .debug_selector(|| "assistant-trace-viewport".to_owned())
            .w_full()
            .max_h(TRACE_VIEWPORT_MAX_HEIGHT)
            .overflow_y_scrollbar()
            .child(rows);
        let trace_view = view.clone();
        let trace_open = self.trace_is_open(message);
        let mut accordion = Accordion::new(("assistant-trace", message_id.0))
            .multiple(false)
            .bordered(false)
            .small()
            .on_toggle_click(move |open_indices, _, cx| {
                trace_view.update(cx, |view, cx| {
                    view.set_trace_disclosure_override(message_id, open_indices.contains(&0), cx);
                });
            });
        accordion = accordion.item(|item| {
            item.open(trace_open)
                .title(self.render_trace_header(message, cx, view))
                .hover(|this| this.bg(cx.theme().accent.opacity(0.24)))
                .child(v_flex().w_full().gap_1().child(trace_viewport))
        });

        accordion.into_any_element()
    }

    fn render_trace_header(&self, message: &Message, cx: &App, view: &Entity<Self>) -> AnyElement {
        let active = Self::trace_is_active(message);
        let final_answer_started = self.trace_is_final_answer_started(message.id);
        let message_id = message.id;
        let label = if active && !final_answer_started {
            ShimmerText::new("Thinking")
                .id(("assistant-trace-thinking", message.id.0))
                .duration(Duration::from_millis(2200))
                .spread(0.38)
                .font_medium()
                .into_any_element()
        } else {
            let text = match message.status {
                MessageStatus::Stopped => "Thinking stopped",
                MessageStatus::Failed => "Thinking failed",
                _ => "Thought",
            };
            div().font_medium().child(text).into_any_element()
        };
        let elapsed = self.trace_elapsed(message);
        let stop_view = view.clone();

        let leading = h_flex()
            .items_center()
            .gap_2()
            .when(active && !final_answer_started, |this| {
                this.child(Spinner::new().xsmall().color(cx.theme().primary))
            })
            .child(label)
            .when_some(elapsed, |this, elapsed| {
                this.child(
                    div()
                        .text_size(rems(0.6875))
                        .text_color(cx.theme().muted_foreground)
                        .child(format_elapsed(elapsed)),
                )
            })
            .into_any_element();

        let mut header = h_flex()
            .w_full()
            .items_center()
            .justify_between()
            .child(leading);
        if active {
            header = header.child(
                Button::new(("stop-assistant-trace", message.id.0))
                    .ghost()
                    .xsmall()
                    .compact()
                    .label("Stop")
                    .on_click(move |_, _, cx| {
                        stop_view.update(cx, |view, cx| view.request_stop(message_id, cx));
                    }),
            );
        }
        header.into_any_element()
    }

    fn render_trace_entry_title(entry: &AssistantTraceEntry, cx: &App) -> AnyElement {
        let status = trace_status_label(entry.status);
        let icon = render_trace_entry_indicator(entry, cx);
        h_flex()
            .items_center()
            .gap_1()
            .child(icon)
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .text_size(rems(0.8125))
                    .child(trace_entry_title(entry)),
            )
            .child(
                div()
                    .text_size(rems(0.6875))
                    .font_medium()
                    .text_color(trace_status_color(entry.status, cx).opacity(0.9))
                    .child(status),
            )
            .into_any_element()
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
            .when(!entry.input.is_empty(), |this| {
                this.child(trace_detail_block("Input", &entry.input, cx))
            })
            .when(!entry.output.is_empty(), |this| {
                this.child(trace_detail_block("Output", &entry.output, cx))
            })
            .when(entry.input.is_empty() && entry.output.is_empty(), |this| {
                this.child(
                    div()
                        .text_size(rems(0.6875))
                        .text_color(cx.theme().muted_foreground)
                        .child("No details available yet."),
                )
            })
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

fn trace_entries_for_timeline(message: &Message) -> Vec<AssistantTraceEntry> {
    let mut entries = message.assistant_trace.entries.clone();
    entries.sort_by_key(|entry| entry.sequence);
    entries.reverse();
    entries
}

fn trace_entry_title(entry: &AssistantTraceEntry) -> String {
    if entry.kind == AssistantTraceKind::ReasoningSummary
        && entry.title.eq_ignore_ascii_case("thinking")
    {
        "Reasoning".to_owned()
    } else {
        entry.title.clone()
    }
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
    .text_color(cx.theme().foreground.opacity(0.9))
    .into_any_element()
}

fn render_trace_entry_indicator(entry: &AssistantTraceEntry, cx: &App) -> AnyElement {
    if matches!(
        entry.status,
        AssistantTraceStatus::Streaming
            | AssistantTraceStatus::Requested
            | AssistantTraceStatus::Running
    ) {
        return Spinner::new()
            .xsmall()
            .color(trace_status_color(entry.status, cx))
            .into_any_element();
    }

    let icon = match (entry.kind, entry.status) {
        (AssistantTraceKind::ReasoningSummary, AssistantTraceStatus::Completed) => {
            Icon::new(IconName::CircleCheck)
        }
        (
            AssistantTraceKind::ReasoningSummary,
            AssistantTraceStatus::Failed
            | AssistantTraceStatus::Rejected
            | AssistantTraceStatus::Stopped,
        ) => Icon::new(IconName::CircleX),
        (AssistantTraceKind::ReasoningSummary, _) => Icon::new(IconName::LoaderCircle),
        (AssistantTraceKind::Tool, _) => Icon::empty().path("icons/agent-wrench.svg"),
    };

    icon.xsmall()
        .text_color(trace_status_color(entry.status, cx))
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
        AssistantTraceStatus::Streaming
        | AssistantTraceStatus::Running
        | AssistantTraceStatus::AwaitingApproval => cx.theme().warning,
        AssistantTraceStatus::Completed => cx.theme().success,
        AssistantTraceStatus::Rejected | AssistantTraceStatus::Stopped => {
            cx.theme().muted_foreground
        }
        AssistantTraceStatus::Failed => cx.theme().danger,
        AssistantTraceStatus::Requested => cx.theme().muted_foreground,
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
