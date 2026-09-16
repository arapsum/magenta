use super::*;

impl ConversationView {
    pub(super) fn tool_name_for_result(&self, assistant_id: MessageId, call_id: &str) -> String {
        self.messages
            .iter()
            .find(|message| message.message.id == assistant_id)
            .and_then(|message| {
                message
                    .message
                    .assistant_trace
                    .entries
                    .iter()
                    .rev()
                    .find(|entry| {
                        entry.kind == AssistantTraceKind::Tool
                            && entry.key == format!("tool:{call_id}")
                    })
            })
            .map_or_else(
                || "workspace tool".to_owned(),
                |entry| {
                    entry
                        .tool_name
                        .clone()
                        .unwrap_or_else(|| "workspace tool".to_owned())
                },
            )
    }

    pub(crate) fn update_trace<F>(
        &mut self,
        generation: u64,
        assistant_id: MessageId,
        update: F,
        cx: &mut Context<'_, Self>,
    ) where
        F: FnOnce(&mut AssistantTrace),
    {
        if self.generation != generation || self.streaming_message != Some(assistant_id) {
            return;
        }
        let Some(message) = self
            .messages
            .iter_mut()
            .find(|message| message.message.id == assistant_id)
        else {
            return;
        };
        update(&mut message.message.assistant_trace);
        self.list_state.remeasure_items(0..self.messages.len());
        cx.notify();
    }

    pub(crate) fn update_reasoning_trace(
        &mut self,
        generation: u64,
        assistant_id: MessageId,
        key: String,
        title: impl Into<String>,
        delta: Option<String>,
        cx: &mut Context<'_, Self>,
    ) {
        self.update_trace(
            generation,
            assistant_id,
            move |trace| {
                let entry = ensure_trace_entry(
                    trace,
                    key,
                    AssistantTraceKind::ReasoningSummary,
                    title.into(),
                    None,
                    AssistantTraceStatus::Streaming,
                );
                if let Some(delta) = delta {
                    append_bounded(&mut entry.output, &delta);
                }
            },
            cx,
        );
    }

    pub(crate) fn complete_reasoning_trace(
        &mut self,
        generation: u64,
        assistant_id: MessageId,
        key: String,
        text: String,
        cx: &mut Context<'_, Self>,
    ) {
        self.update_trace(
            generation,
            assistant_id,
            move |trace| {
                let entry = ensure_trace_entry(
                    trace,
                    key,
                    AssistantTraceKind::ReasoningSummary,
                    "Thinking".to_owned(),
                    None,
                    AssistantTraceStatus::Completed,
                );
                if !text.is_empty() {
                    entry.output = text;
                }
                entry.status = AssistantTraceStatus::Completed;
                entry.finished_at = Some(trace_now());
            },
            cx,
        );
    }

    pub(super) fn update_tool_trace(
        &mut self,
        generation: u64,
        assistant_id: MessageId,
        update: ToolTraceUpdate,
        cx: &mut Context<'_, Self>,
    ) {
        let ToolTraceUpdate {
            call_id,
            tool_name,
            status,
            input,
            output,
        } = update;
        let key = format!("tool:{call_id}");
        self.update_trace(
            generation,
            assistant_id,
            move |trace| {
                let entry = ensure_trace_entry(
                    trace,
                    key,
                    AssistantTraceKind::Tool,
                    tool_name.clone(),
                    Some(tool_name.clone()),
                    status,
                );
                if let Some(input) = input
                    && (entry.input.is_empty() || status == AssistantTraceStatus::Requested)
                {
                    entry.input = input;
                }
                if let Some(output) = output {
                    entry.output = bounded_text(&output);
                }
                entry.status = status;
                if matches!(
                    status,
                    AssistantTraceStatus::Completed
                        | AssistantTraceStatus::Rejected
                        | AssistantTraceStatus::Failed
                        | AssistantTraceStatus::Stopped
                ) {
                    entry.finished_at = Some(trace_now());
                }
            },
            cx,
        );
    }

    pub(super) fn append_tool_trace_output(
        &mut self,
        generation: u64,
        assistant_id: MessageId,
        call_id: &str,
        chunk: &str,
        cx: &mut Context<'_, Self>,
    ) {
        let key = format!("tool:{call_id}");
        self.update_trace(
            generation,
            assistant_id,
            move |trace| {
                let entry = ensure_trace_entry(
                    trace,
                    key,
                    AssistantTraceKind::Tool,
                    "run_command".to_owned(),
                    Some("run_command".to_owned()),
                    AssistantTraceStatus::Running,
                );
                append_bounded(&mut entry.output, chunk);
                entry.status = AssistantTraceStatus::Running;
            },
            cx,
        );
    }

    pub(crate) fn decide_agent_approval(
        &mut self,
        decision: AgentApprovalDecision,
        cx: &mut Context<'_, Self>,
    ) {
        let Some((_, approval)) = self.pending_agent_approval.take() else {
            return;
        };
        if let Some(controller) = &self.agent_controller {
            let _ = controller.decide(approval.request_id, decision);
        }
        cx.notify();
    }
}

pub(super) fn append_bounded(output: &mut String, chunk: &str) {
    const MAX_LIVE_OUTPUT_BYTES: usize = 128 * 1024;

    output.push_str(chunk);
    if output.len() <= MAX_LIVE_OUTPUT_BYTES {
        return;
    }

    let mut start = output.len() - MAX_LIVE_OUTPUT_BYTES;
    while !output.is_char_boundary(start) {
        start += 1;
    }
    output.drain(..start);
}

fn trace_now() -> Timestamp {
    Timestamp(chrono::Local::now().timestamp_millis())
}

fn bounded_text(value: &str) -> String {
    let mut bounded = String::new();
    append_bounded(&mut bounded, value);
    bounded
}

fn ensure_trace_entry(
    trace: &mut AssistantTrace,
    key: String,
    kind: AssistantTraceKind,
    title: String,
    tool_name: Option<String>,
    status: AssistantTraceStatus,
) -> &mut AssistantTraceEntry {
    if let Some(index) = trace.entries.iter().position(|entry| entry.key == key) {
        let entry = &mut trace.entries[index];
        if entry.title.is_empty() {
            entry.title = title;
        }
        if entry.tool_name.is_none() {
            entry.tool_name = tool_name;
        }
        if entry.started_at.is_none() {
            entry.started_at = Some(trace_now());
        }
        return entry;
    }

    let sequence = trace
        .entries
        .iter()
        .map(|entry| entry.sequence)
        .max()
        .map_or(0, |sequence| sequence.saturating_add(1));
    trace.entries.push(AssistantTraceEntry {
        key,
        sequence,
        kind,
        status,
        title,
        tool_name,
        input: String::new(),
        output: String::new(),
        started_at: Some(trace_now()),
        finished_at: None,
    });
    trace.entries.last_mut().expect("trace entry was inserted")
}
