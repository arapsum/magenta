use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use futures_util::StreamExt as _;
use magenta_core::{
    AgentRunEvent, AgentToolCall, AgentToolOutput, AssistantTextPhase, AssistantTrace,
    AssistantTraceEntry, AssistantTraceKind, AssistantTraceStatus, ConversationStore,
    GenerationEvent, GenerationStream, MessageId, Timestamp, WorkspaceCommandResult,
};

#[derive(Clone)]
pub(crate) struct AssistantTraceRecorder {
    store: Arc<dyn ConversationStore>,
    message_id: MessageId,
    state: Arc<Mutex<TraceState>>,
}

struct TraceState {
    trace: AssistantTrace,
    started_at: Instant,
}

impl AssistantTraceRecorder {
    pub(crate) fn new(
        store: Arc<dyn ConversationStore>,
        message_id: MessageId,
        initial: AssistantTrace,
    ) -> Self {
        Self {
            store,
            message_id,
            state: Arc::new(Mutex::new(TraceState {
                trace: initial,
                started_at: Instant::now(),
            })),
        }
    }

    pub(crate) async fn observe_generation(&self, event: &GenerationEvent) {
        match event {
            GenerationEvent::ReasoningSummaryStarted { key, title } => {
                self.start_reasoning(key, title).await;
            }
            GenerationEvent::ReasoningSummaryDelta { key, delta } => {
                self.append_reasoning(key, delta).await;
            }
            GenerationEvent::ReasoningSummaryCompleted { key, text } => {
                self.complete_reasoning(key, text).await;
            }
            GenerationEvent::Completed(_) => {
                self.finish(AssistantTraceStatus::Completed).await;
            }
            GenerationEvent::Started
            | GenerationEvent::TextDelta(_)
            | GenerationEvent::TextDeltaWithPhase {
                phase: AssistantTextPhase::Commentary,
                ..
            }
            | GenerationEvent::TextDeltaWithPhase {
                phase: AssistantTextPhase::FinalAnswer,
                ..
            } => {}
        }
    }

    pub(crate) async fn observe_agent(&self, event: &AgentRunEvent) {
        match event {
            AgentRunEvent::ReasoningSummaryStarted { key, title } => {
                self.start_reasoning(key, title).await;
            }
            AgentRunEvent::ReasoningSummaryDelta { key, delta } => {
                self.append_reasoning(key, delta).await;
            }
            AgentRunEvent::ReasoningSummaryCompleted { key, text } => {
                self.complete_reasoning(key, text).await;
            }
            AgentRunEvent::ToolCall(call) => self.request_tool(call).await,
            AgentRunEvent::ApprovalRequired(approval) => {
                self.set_tool_status(
                    &approval.tool_call_id,
                    AssistantTraceStatus::AwaitingApproval,
                    None,
                    None,
                    Some(approval.reason.clone()),
                    true,
                )
                .await;
            }
            AgentRunEvent::CommandStarted { call_id, command } => {
                self.set_tool_status(
                    call_id,
                    AssistantTraceStatus::Running,
                    Some(command.display()),
                    Some(command.display()),
                    None,
                    true,
                )
                .await;
            }
            AgentRunEvent::ToolResult(output) => self.complete_tool(output).await,
            AgentRunEvent::Completed(_) => self.finish(AssistantTraceStatus::Completed).await,
            AgentRunEvent::Started
            | AgentRunEvent::TextDelta(_)
            | AgentRunEvent::TextDeltaWithPhase { .. }
            | AgentRunEvent::WorkspaceChange(_)
            | AgentRunEvent::WorkspaceInvalidated => {}
            AgentRunEvent::CommandOutput { call_id, chunk, .. } => {
                self.append_tool_output(call_id, chunk).await;
            }
        }
    }

    async fn start_reasoning(&self, key: &str, title: &str) {
        self.update(true, |trace| {
            ensure_entry(
                trace,
                key,
                AssistantTraceKind::ReasoningSummary,
                AssistantTraceStatus::Streaming,
                title,
                None,
                String::new(),
            );
        })
        .await;
    }

    async fn append_reasoning(&self, key: &str, delta: &str) {
        self.update(false, |trace| {
            let entry = ensure_entry(
                trace,
                key,
                AssistantTraceKind::ReasoningSummary,
                AssistantTraceStatus::Streaming,
                "Thinking",
                None,
                String::new(),
            );
            entry.output = bound_output(&format!("{}{}", entry.output, delta));
        })
        .await;
    }

    async fn complete_reasoning(&self, key: &str, text: &str) {
        self.update(true, |trace| {
            let entry = ensure_entry(
                trace,
                key,
                AssistantTraceKind::ReasoningSummary,
                AssistantTraceStatus::Streaming,
                "Thinking",
                None,
                String::new(),
            );
            if !text.is_empty() {
                text.clone_into(&mut entry.output);
            }
            entry.status = AssistantTraceStatus::Completed;
            entry.finished_at = Some(timestamp());
        })
        .await;
    }

    async fn request_tool(&self, call: &AgentToolCall) {
        self.update(true, |trace| {
            let entry = ensure_entry(
                trace,
                &format!("tool:{}", call.id),
                AssistantTraceKind::Tool,
                AssistantTraceStatus::Requested,
                &tool_title(&call.name),
                Some(call.name.clone()),
                call.arguments.clone(),
            );
            entry.input.clone_from(&call.arguments);
        })
        .await;
    }

    async fn set_tool_status(
        &self,
        call_id: &str,
        status: AssistantTraceStatus,
        title: Option<String>,
        input: Option<String>,
        output: Option<String>,
        persist: bool,
    ) {
        self.update(persist, |trace| {
            let entry = ensure_entry(
                trace,
                &format!("tool:{call_id}"),
                AssistantTraceKind::Tool,
                status,
                title.as_deref().unwrap_or("Tool"),
                None,
                input.clone().unwrap_or_default(),
            );
            entry.status = status;
            if let Some(title) = title.as_deref().filter(|title| !title.is_empty()) {
                title.clone_into(&mut entry.title);
            }
            if let Some(input) = input {
                entry.input = input;
            }
            if let Some(output) = output {
                entry.output = output;
            }
            if matches!(
                status,
                AssistantTraceStatus::Completed
                    | AssistantTraceStatus::Rejected
                    | AssistantTraceStatus::Failed
                    | AssistantTraceStatus::Stopped
            ) {
                entry.finished_at = Some(timestamp());
            }
        })
        .await;
    }

    async fn complete_tool(&self, output: &AgentToolOutput) {
        let status = if !output.is_error {
            AssistantTraceStatus::Completed
        } else if output.output.to_ascii_lowercase().contains("reject") {
            AssistantTraceStatus::Rejected
        } else {
            AssistantTraceStatus::Failed
        };
        let output_text = serde_json::from_str::<WorkspaceCommandResult>(&output.output)
            .ok()
            .map_or_else(
                || output.output.clone(),
                |result| match (result.stdout.is_empty(), result.stderr.is_empty()) {
                    (false, false) => format!("{}\n{}", result.stdout, result.stderr),
                    (false, true) => result.stdout,
                    (true, false) => result.stderr,
                    (true, true) => output.output.clone(),
                },
            );
        self.set_tool_status(
            &output.call_id,
            status,
            None,
            None,
            Some(bound_output(&output_text)),
            true,
        )
        .await;
    }

    async fn append_tool_output(&self, call_id: &str, chunk: &str) {
        self.update(false, |trace| {
            let entry = ensure_entry(
                trace,
                &format!("tool:{call_id}"),
                AssistantTraceKind::Tool,
                AssistantTraceStatus::Running,
                "run_command",
                Some("run_command".to_owned()),
                String::new(),
            );
            entry.output = bound_output(&format!("{}{}", entry.output, chunk));
            entry.status = AssistantTraceStatus::Running;
        })
        .await;
    }

    async fn finish(&self, terminal_status: AssistantTraceStatus) {
        self.update(true, |trace| {
            for entry in &mut trace.entries {
                if matches!(
                    entry.status,
                    AssistantTraceStatus::Streaming
                        | AssistantTraceStatus::Requested
                        | AssistantTraceStatus::Running
                        | AssistantTraceStatus::AwaitingApproval
                ) {
                    entry.status = terminal_status;
                    entry.finished_at = Some(timestamp());
                }
            }
        })
        .await;
    }

    async fn update(&self, persist: bool, update: impl FnOnce(&mut AssistantTrace)) {
        let trace = {
            let Ok(mut state) = self.state.lock() else {
                return;
            };
            update(&mut state.trace);
            state.trace.thinking_duration_ms = Some(duration_millis(state.started_at.elapsed()));
            state.trace.clone()
        };

        if persist {
            let _ = self
                .store
                .upsert_assistant_trace(self.message_id, trace)
                .await;
        }
    }
}

pub(crate) fn traced_generation_stream(
    store: Arc<dyn ConversationStore>,
    message_id: MessageId,
    initial: AssistantTrace,
    stream: GenerationStream,
) -> GenerationStream {
    let recorder = AssistantTraceRecorder::new(store, message_id, initial);
    Box::pin(async_stream::try_stream! {
        futures_util::pin_mut!(stream);
        while let Some(event) = stream.next().await {
            match event {
                Ok(event) => {
                    recorder.observe_generation(&event).await;
                    yield event;
                }
                Err(error) => {
                    recorder.finish(AssistantTraceStatus::Failed).await;
                    Err::<(), _>(error)?;
                }
            }
        }
    })
}

fn ensure_entry<'a>(
    trace: &'a mut AssistantTrace,
    key: &str,
    kind: AssistantTraceKind,
    status: AssistantTraceStatus,
    title: &str,
    tool_name: Option<String>,
    input: String,
) -> &'a mut AssistantTraceEntry {
    if let Some(index) = trace.entries.iter().position(|entry| entry.key == key) {
        return &mut trace.entries[index];
    }
    let sequence = trace
        .entries
        .iter()
        .map(|entry| entry.sequence)
        .max()
        .map_or(0, |sequence| sequence.saturating_add(1));
    trace.entries.push(AssistantTraceEntry {
        key: key.to_owned(),
        sequence,
        kind,
        status,
        title: title.to_owned(),
        tool_name,
        input,
        output: String::new(),
        started_at: Some(timestamp()),
        finished_at: None,
    });
    trace.entries.last_mut().expect("entry was just inserted")
}

fn tool_title(name: &str) -> String {
    name.split('_')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            chars.next().map_or_else(String::new, |first| {
                format!("{}{}", first.to_ascii_uppercase(), chars.as_str())
            })
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn bound_output(output: &str) -> String {
    const MAX_OUTPUT_BYTES: usize = 128 * 1024;
    if output.len() <= MAX_OUTPUT_BYTES {
        return output.to_owned();
    }
    let mut start = output.len() - MAX_OUTPUT_BYTES;
    while !output.is_char_boundary(start) {
        start += 1;
    }
    format!("…{}", &output[start..])
}

fn timestamp() -> Timestamp {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_millis();
    Timestamp(i64::try_from(millis).unwrap_or(i64::MAX))
}

fn duration_millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}
