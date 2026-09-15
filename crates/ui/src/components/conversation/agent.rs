use futures_util::StreamExt as _;
use magenta_application::PendingAgentGeneration;
use magenta_core::{
    AgentApprovalDecision, AgentApprovalSubject, AgentRunEvent, AgentRunStream, AgentToolOutput,
    AssistantTrace, AssistantTraceEntry, AssistantTraceKind, AssistantTraceStatus, MessageId,
    ProviderError, ProviderId, Timestamp, WorkspaceCommandOutputStream, WorkspaceCommandResult,
};

use super::*;

enum AgentStreamControl {
    Continue,
    Completed(GenerationOutcome),
}

struct ToolTraceUpdate {
    call_id: String,
    tool_name: String,
    status: AssistantTraceStatus,
    input: Option<String>,
    output: Option<String>,
}

impl ConversationView {
    pub(crate) fn apply_workspace_review(&mut self, window: &Window, cx: &Context<'_, Self>) {
        let Some(controller) = self.agent_controller.clone() else {
            return;
        };

        self.generation_task = Some(cx.spawn_in(window, async move |view, window| {
            let result = controller.apply_workspace_changes().await;

            _ = view.update_in(window, |view, _, cx| {
                if result.is_ok() {
                    view.agent_controller.take();
                    cx.emit(ConversationViewEvent::WorkspaceInvalidated);
                }
                cx.notify();
            });
        }));
    }

    pub(crate) fn discard_workspace_review(&mut self, window: &Window, cx: &Context<'_, Self>) {
        let Some(controller) = self.agent_controller.clone() else {
            return;
        };

        self.generation_task = Some(cx.spawn_in(window, async move |view, window| {
            let result = controller.discard_workspace_changes().await;

            _ = view.update_in(window, |view, _, cx| {
                if result.is_ok() {
                    view.agent_controller.take();
                }
                cx.notify();
            });
        }));
    }

    pub(crate) fn start_agent_generation(
        &mut self,
        pending: PendingAgentGeneration,
        window: &Window,
        cx: &mut Context<'_, Self>,
    ) {
        let PendingAgentGeneration {
            conversation,
            user_message,
            assistant_message,
            stream,
            controller,
            user_sequence,
            assistant_sequence,
            context_report,
        } = pending;
        let provider_id = conversation.generation.provider.clone();

        self.cancel_generation(cx);

        let user = Self::rendered_message(user_message, cx);
        if self.conversation.is_some() {
            self.origins
                .insert(assistant_message.id, conversation.generation);
        }

        let assistant = Self::rendered_message(assistant_message, cx);
        let old_count = self.messages.len();
        let assistant_id = assistant.message.id;

        self.messages.push(user);
        self.messages.push(assistant);

        self.set_pending_metadata(
            self.messages[old_count].message.id,
            user_sequence,
            assistant_id,
            assistant_sequence,
            context_report.omitted_messages,
            cx,
        );

        if self.trim_oldest_to_limit(cx) == 0 {
            self.list_state.splice(old_count..old_count, 2);
        } else {
            self.list_state
                .reset_with_uniform_height(self.messages.len(), px(96.));
        }
        self.list_state.set_follow_mode(FollowMode::Tail);
        self.list_state.scroll_to_end();
        self.agent_controller = Some(controller);
        self.begin_agent_stream(assistant_id, provider_id, stream, window, cx);
        cx.notify();
    }

    pub(crate) fn start_agent_retry(
        &mut self,
        pending: PendingAgentGeneration,
        window: &Window,
        cx: &mut Context<'_, Self>,
    ) {
        let PendingAgentGeneration {
            conversation,
            assistant_message,
            stream,
            controller,
            assistant_sequence,
            context_report,
            ..
        } = pending;

        self.cancel_generation(cx);
        self.conversation = Some(conversation.clone());
        self.origins
            .insert(assistant_message.id, conversation.generation.clone());

        let assistant = Self::rendered_message(assistant_message, cx);
        let old_count = self.messages.len();
        let assistant_id = assistant.message.id;

        self.messages.push(assistant);

        self.set_pending_metadata(
            MessageId(0),
            magenta_core::MessageSequence(0),
            assistant_id,
            assistant_sequence,
            context_report.omitted_messages,
            cx,
        );

        if self.trim_oldest_to_limit(cx) == 0 {
            self.list_state.splice(old_count..old_count, 1);
        } else {
            self.list_state
                .reset_with_uniform_height(self.messages.len(), gpui_kit::px(96.));
        }
        self.list_state.set_follow_mode(gpui_kit::FollowMode::Tail);
        self.list_state.scroll_to_end();
        self.agent_controller = Some(controller);
        self.begin_agent_stream(
            assistant_id,
            conversation.generation.provider,
            stream,
            window,
            cx,
        );
        cx.notify();
    }

    fn begin_agent_stream(
        &mut self,
        assistant_id: MessageId,
        provider_id: ProviderId,
        stream: AgentRunStream,
        window: &Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;

        self.streaming_message = Some(assistant_id);
        self.generation_progress = Some(GenerationProgress::new(
            assistant_id,
            provider_id.clone(),
            self.origins.get(&assistant_id).cloned(),
        ));

        self.start_generation_clock(generation, assistant_id, window, cx);
        self.generation_task = Some(cx.spawn_in(window, async move |view, window| {
            let mut completed = None;
            let mut stream = stream;

            while let Some(event) = stream.next().await {
                let event = match event {
                    Ok(event) => event,
                    Err(error) => {
                        _ = view.update_in(window, |view, window, cx| {
                            view.fail_stream(generation, assistant_id, &error, window, cx);
                        });
                        return;
                    }
                };
                let Ok(control) = view.update_in(window, |view, _, cx| {
                    view.apply_agent_event(generation, assistant_id, event, cx)
                }) else {
                    return;
                };
                if let AgentStreamControl::Completed(outcome) = control {
                    completed = Some(outcome);
                    break;
                }
            }

            if let Some(outcome) = completed {
                _ = view.update_in(window, |view, _, cx| {
                    view.finish_stream(generation, assistant_id, outcome, cx);
                });
            } else {
                let error = ProviderError::with_kind(
                    provider_id,
                    magenta_core::ProviderErrorKind::IncompleteResponse,
                    IncompleteGeneration,
                );
                _ = view.update_in(window, |view, window, cx| {
                    view.fail_stream(generation, assistant_id, &error, window, cx);
                });
            }
        }));
        cx.emit(ConversationViewEvent::GenerationStarted);
    }

    fn apply_agent_event(
        &mut self,
        generation: u64,
        assistant_id: MessageId,
        event: AgentRunEvent,
        cx: &mut Context<'_, Self>,
    ) -> AgentStreamControl {
        match event {
            AgentRunEvent::Started => self.mark_provider_started(generation, assistant_id, cx),
            AgentRunEvent::TextDelta(chunk) => {
                self.push_stream_chunk_phase(generation, assistant_id, &chunk, None, cx);
            }
            AgentRunEvent::TextDeltaWithPhase { delta, phase } => {
                self.push_stream_chunk_phase(generation, assistant_id, &delta, Some(phase), cx);
            }
            AgentRunEvent::ReasoningSummaryStarted { key, title } => {
                self.update_reasoning_trace(generation, assistant_id, key, title, None, cx);
            }
            AgentRunEvent::ReasoningSummaryDelta { key, delta } => {
                self.update_reasoning_trace(
                    generation,
                    assistant_id,
                    key,
                    "Thinking",
                    Some(delta),
                    cx,
                );
            }
            AgentRunEvent::ReasoningSummaryCompleted { key, text } => {
                self.complete_reasoning_trace(generation, assistant_id, key, text, cx);
            }
            AgentRunEvent::ToolCall(call) => {
                self.update_tool_trace(
                    generation,
                    assistant_id,
                    ToolTraceUpdate {
                        call_id: call.id,
                        tool_name: call.name,
                        status: AssistantTraceStatus::Requested,
                        input: Some(call.arguments),
                        output: None,
                    },
                    cx,
                );
            }
            AgentRunEvent::ApprovalRequired(approval) => {
                self.apply_approval_event(generation, assistant_id, approval, cx);
            }
            AgentRunEvent::ToolResult(result) => {
                self.apply_tool_result_event(generation, assistant_id, &result, cx);
            }
            AgentRunEvent::WorkspaceChange(change) => {
                cx.emit(ConversationViewEvent::WorkspaceChange(change));
            }
            AgentRunEvent::CommandStarted { call_id, command } => {
                self.live_commands.insert(
                    (assistant_id, call_id.clone()),
                    LiveCommand {
                        stdout: String::new(),
                        stderr: String::new(),
                        result: None,
                    },
                );
                self.update_tool_trace(
                    generation,
                    assistant_id,
                    ToolTraceUpdate {
                        call_id,
                        tool_name: "run_command".to_owned(),
                        status: AssistantTraceStatus::Running,
                        input: Some(command.display()),
                        output: None,
                    },
                    cx,
                );
                self.list_state.remeasure_items(0..self.messages.len());
                cx.notify();
            }
            AgentRunEvent::CommandOutput {
                call_id,
                stream,
                chunk,
            } => {
                let updated = if let Some(command) =
                    self.live_commands.get_mut(&(assistant_id, call_id.clone()))
                {
                    let output = match stream {
                        WorkspaceCommandOutputStream::Stdout => &mut command.stdout,
                        WorkspaceCommandOutputStream::Stderr => &mut command.stderr,
                    };
                    append_bounded(output, &chunk);
                    true
                } else {
                    false
                };
                if updated {
                    self.append_tool_trace_output(generation, assistant_id, &call_id, &chunk, cx);
                    self.list_state.remeasure_items(0..self.messages.len());
                    cx.notify();
                }
            }
            AgentRunEvent::WorkspaceInvalidated => {
                cx.emit(ConversationViewEvent::WorkspaceInvalidated);
            }
            AgentRunEvent::Completed(outcome) => return AgentStreamControl::Completed(outcome),
        }
        AgentStreamControl::Continue
    }

    fn apply_approval_event(
        &mut self,
        generation: u64,
        assistant_id: MessageId,
        approval: magenta_core::AgentApprovalRequest,
        cx: &mut Context<'_, Self>,
    ) {
        self.pending_agent_approval = Some((assistant_id, approval.clone()));

        let detail = match &approval.subject {
            AgentApprovalSubject::Workspace { diff, .. } => diff.clone().unwrap_or_default(),
            AgentApprovalSubject::Command(command) => command.display(),
        };
        self.update_tool_trace(
            generation,
            assistant_id,
            ToolTraceUpdate {
                call_id: approval.tool_call_id,
                tool_name: approval.tool_name,
                status: AssistantTraceStatus::AwaitingApproval,
                input: Some(detail),
                output: Some(approval.reason),
            },
            cx,
        );
    }

    fn apply_tool_result_event(
        &mut self,
        generation: u64,
        assistant_id: MessageId,
        result: &AgentToolOutput,
        cx: &mut Context<'_, Self>,
    ) {
        let tool_name = self.tool_name_for_result(assistant_id, &result.call_id);

        if tool_name == "run_command"
            && let Ok(command_result) =
                serde_json::from_str::<WorkspaceCommandResult>(&result.output)
            && let Some(command) = self
                .live_commands
                .get_mut(&(assistant_id, result.call_id.clone()))
        {
            command.stdout.clone_from(&command_result.stdout);
            command.stderr.clone_from(&command_result.stderr);
            command.result = Some(command_result);
        }

        self.pending_agent_approval = None;
        let output = if tool_name == "run_command" {
            self.live_commands
                .get(&(assistant_id, result.call_id.clone()))
                .map_or_else(
                    || result.output.clone(),
                    |command| match (command.stdout.is_empty(), command.stderr.is_empty()) {
                        (false, false) => format!("{}\n{}", command.stdout, command.stderr),
                        (false, true) => command.stdout.clone(),
                        (true, false) => command.stderr.clone(),
                        (true, true) => result.output.clone(),
                    },
                )
        } else {
            result.output.clone()
        };
        let status = if result.is_error {
            if result.output.to_ascii_lowercase().contains("reject") {
                AssistantTraceStatus::Rejected
            } else {
                AssistantTraceStatus::Failed
            }
        } else {
            AssistantTraceStatus::Completed
        };
        self.update_tool_trace(
            generation,
            assistant_id,
            ToolTraceUpdate {
                call_id: result.call_id.clone(),
                tool_name,
                status,
                input: Some(output),
                output: None,
            },
            cx,
        );
    }

    fn tool_name_for_result(&self, assistant_id: MessageId, call_id: &str) -> String {
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

    fn update_trace<F>(
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

    pub(super) fn update_reasoning_trace(
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

    pub(super) fn complete_reasoning_trace(
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

    fn update_tool_trace(
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

    fn append_tool_trace_output(
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

fn append_bounded(output: &mut String, chunk: &str) {
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
