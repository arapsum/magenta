use std::time::Instant;

use magenta_core::{
    AgentRunEvent, AgentToolOutput, AssistantTextPhase, AssistantTrace, AssistantTraceEntry,
    AssistantTraceKind, AssistantTraceStatus, GenerationEvent, MessageId, MessageStatus,
    WorkspaceCommandOutputStream, WorkspaceCommandResult,
};

use super::super::{ResponseRun, ResponseRunCoordinator};
use super::{RunEventResult, now};
use crate::components::conversation::{GenerationPhase, LiveCommand};

impl ResponseRunCoordinator {
    pub(crate) fn apply_generation_event(
        &mut self,
        assistant_id: MessageId,
        event: GenerationEvent,
    ) -> RunEventResult {
        self.apply_generation_event_state(assistant_id, event)
    }

    pub(crate) fn apply_agent_event(
        &mut self,
        assistant_id: MessageId,
        event: AgentRunEvent,
    ) -> RunEventResult {
        self.apply_agent_event_state(assistant_id, event)
    }

    fn apply_generation_event_state(
        &mut self,
        assistant_id: MessageId,
        event: GenerationEvent,
    ) -> RunEventResult {
        let Some(run) = self.runs.get_mut(&assistant_id) else {
            return RunEventResult::Continue;
        };

        match event {
            GenerationEvent::Started => mark_provider_started(run),
            GenerationEvent::TextDelta(delta) => {
                append_text(run, &delta, Some(AssistantTextPhase::FinalAnswer));
            }
            GenerationEvent::TextDeltaWithPhase { delta, phase } => {
                append_text(run, &delta, Some(phase));
            }
            GenerationEvent::ReasoningSummaryStarted { key, title } => {
                update_reasoning(
                    &mut run.assistant.message.assistant_trace,
                    &key,
                    &title,
                    None,
                );
            }
            GenerationEvent::ReasoningSummaryDelta { key, delta } => {
                update_reasoning(
                    &mut run.assistant.message.assistant_trace,
                    &key,
                    "Thinking",
                    Some(&delta),
                );
            }
            GenerationEvent::ReasoningSummaryCompleted { key, text } => {
                complete_reasoning(&mut run.assistant.message.assistant_trace, &key, &text);
            }
            GenerationEvent::Completed(outcome) => return RunEventResult::Completed(outcome),
        }

        RunEventResult::Continue
    }

    fn apply_agent_event_state(
        &mut self,
        assistant_id: MessageId,
        event: AgentRunEvent,
    ) -> RunEventResult {
        let Some(run) = self.runs.get_mut(&assistant_id) else {
            return RunEventResult::Continue;
        };

        match event {
            AgentRunEvent::Started => mark_provider_started(run),
            AgentRunEvent::TextDelta(delta) => append_text(run, &delta, None),
            AgentRunEvent::TextDeltaWithPhase { delta, phase } => {
                append_text(run, &delta, Some(phase));
            }
            AgentRunEvent::ReasoningSummaryStarted { key, title } => {
                update_reasoning(
                    &mut run.assistant.message.assistant_trace,
                    &key,
                    &title,
                    None,
                );
            }
            AgentRunEvent::ReasoningSummaryDelta { key, delta } => update_reasoning(
                &mut run.assistant.message.assistant_trace,
                &key,
                "Thinking",
                Some(&delta),
            ),
            AgentRunEvent::ReasoningSummaryCompleted { key, text } => {
                complete_reasoning(&mut run.assistant.message.assistant_trace, &key, &text);
            }
            AgentRunEvent::ToolCall(call) => update_tool(
                &mut run.assistant.message.assistant_trace,
                &call.id,
                &call.name,
                AssistantTraceStatus::Requested,
                Some(call.arguments),
                None,
            ),
            AgentRunEvent::ApprovalRequired(approval) => {
                let detail = match &approval.subject {
                    magenta_core::AgentApprovalSubject::Workspace { diff, .. } => {
                        diff.clone().unwrap_or_default()
                    }
                    magenta_core::AgentApprovalSubject::Command(command) => command.display(),
                };

                run.pending_approval = Some((assistant_id, approval.clone()));
                update_tool(
                    &mut run.assistant.message.assistant_trace,
                    &approval.tool_call_id,
                    &approval.tool_name,
                    AssistantTraceStatus::AwaitingApproval,
                    Some(detail),
                    Some(approval.reason),
                );
            }
            AgentRunEvent::ToolResult(result) => apply_tool_result(run, &result),
            AgentRunEvent::WorkspaceChange(change) => {
                return RunEventResult::WorkspaceChange(change);
            }
            AgentRunEvent::CommandStarted { call_id, command } => {
                run.live_commands.insert(
                    (assistant_id, call_id.clone()),
                    LiveCommand {
                        stdout: String::new(),
                        stderr: String::new(),
                        result: None,
                    },
                );
                update_tool(
                    &mut run.assistant.message.assistant_trace,
                    &call_id,
                    "run_command",
                    AssistantTraceStatus::Running,
                    Some(command.display()),
                    None,
                );
            }
            AgentRunEvent::CommandOutput {
                call_id,
                stream,
                chunk,
            } => {
                if let Some(command) = run.live_commands.get_mut(&(assistant_id, call_id.clone())) {
                    let output = match stream {
                        WorkspaceCommandOutputStream::Stdout => &mut command.stdout,
                        WorkspaceCommandOutputStream::Stderr => &mut command.stderr,
                    };
                    append_bounded(output, &chunk);
                }
                append_tool_output(&mut run.assistant.message.assistant_trace, &call_id, &chunk);
            }
            AgentRunEvent::WorkspaceInvalidated => return RunEventResult::WorkspaceInvalidated,
            AgentRunEvent::Completed(outcome) => return RunEventResult::Completed(outcome),
        }

        RunEventResult::Continue
    }
}

fn mark_provider_started(run: &mut ResponseRun) {
    if run.progress.provider_started_at.is_none() {
        run.progress.provider_started_at = Some(Instant::now());
        run.progress.phase = GenerationPhase::Thinking;
    }
}

fn append_text(run: &mut ResponseRun, chunk: &str, phase: Option<AssistantTextPhase>) {
    if run.progress.first_text_at.is_none() {
        run.progress.first_text_at = Some(Instant::now());
    }
    if phase == Some(AssistantTextPhase::FinalAnswer) {
        run.progress.final_answer_started = true;
        run.progress.phase = GenerationPhase::Responding;
    }
    run.assistant.message.content.push_str(chunk);
    run.assistant.message.status = MessageStatus::Streaming;
}

fn update_reasoning(trace: &mut AssistantTrace, key: &str, title: &str, delta: Option<&str>) {
    let entry = ensure_entry(
        trace,
        key,
        AssistantTraceKind::ReasoningSummary,
        AssistantTraceStatus::Streaming,
        title,
        None,
    );

    if let Some(delta) = delta {
        append_bounded(&mut entry.output, delta);
    }
}

fn complete_reasoning(trace: &mut AssistantTrace, key: &str, text: &str) {
    let entry = ensure_entry(
        trace,
        key,
        AssistantTraceKind::ReasoningSummary,
        AssistantTraceStatus::Streaming,
        "Thinking",
        None,
    );

    if !text.is_empty() {
        text.clone_into(&mut entry.output);
    }
    entry.status = AssistantTraceStatus::Completed;
    entry.finished_at = Some(now());
}

fn update_tool(
    trace: &mut AssistantTrace,
    call_id: &str,
    tool_name: &str,
    status: AssistantTraceStatus,
    input: Option<String>,
    output: Option<String>,
) {
    let key = format!("tool:{call_id}");
    let entry = ensure_entry(
        trace,
        &key,
        AssistantTraceKind::Tool,
        status,
        tool_name,
        Some(tool_name.to_owned()),
    );

    if let Some(input) = input
        && (entry.input.is_empty() || status == AssistantTraceStatus::Requested)
    {
        entry.input = input;
    }
    if let Some(output) = output {
        entry.output = output;
    }
    entry.status = status;
    if matches!(
        status,
        AssistantTraceStatus::Completed
            | AssistantTraceStatus::Rejected
            | AssistantTraceStatus::Failed
            | AssistantTraceStatus::Stopped
    ) {
        entry.finished_at = Some(now());
    }
}

fn apply_tool_result(run: &mut ResponseRun, result: &AgentToolOutput) {
    let tool_name = run
        .assistant
        .message
        .assistant_trace
        .entries
        .iter()
        .find(|entry| entry.key == format!("tool:{}", result.call_id))
        .and_then(|entry| entry.tool_name.clone())
        .unwrap_or_else(|| "workspace tool".to_owned());

    if tool_name == "run_command"
        && let Ok(command_result) = serde_json::from_str::<WorkspaceCommandResult>(&result.output)
        && let Some(command) = run
            .live_commands
            .get_mut(&(run.assistant.message.id, result.call_id.clone()))
    {
        command.stdout.clone_from(&command_result.stdout);
        command.stderr.clone_from(&command_result.stderr);
        command.result = Some(command_result);
    }

    run.pending_approval = None;
    let output = if tool_name == "run_command" {
        run.live_commands
            .get(&(run.assistant.message.id, result.call_id.clone()))
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

    update_tool(
        &mut run.assistant.message.assistant_trace,
        &result.call_id,
        &tool_name,
        status,
        Some(bound_text(&output)),
        None,
    );
}

fn append_tool_output(trace: &mut AssistantTrace, call_id: &str, chunk: &str) {
    let key = format!("tool:{call_id}");
    let entry = ensure_entry(
        trace,
        &key,
        AssistantTraceKind::Tool,
        AssistantTraceStatus::Running,
        "run_command",
        Some("run_command".to_owned()),
    );
    append_bounded(&mut entry.output, chunk);
    entry.status = AssistantTraceStatus::Running;
}

fn ensure_entry<'a>(
    trace: &'a mut AssistantTrace,
    key: &str,
    kind: AssistantTraceKind,
    status: AssistantTraceStatus,
    title: &str,
    tool_name: Option<String>,
) -> &'a mut AssistantTraceEntry {
    if let Some(index) = trace.entries.iter().position(|entry| entry.key == key) {
        let entry = &mut trace.entries[index];
        if entry.title.is_empty() {
            title.clone_into(&mut entry.title);
        }
        if entry.tool_name.is_none() {
            entry.tool_name = tool_name;
        }
        if entry.started_at.is_none() {
            entry.started_at = Some(now());
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
        key: key.to_owned(),
        sequence,
        kind,
        status,
        title: title.to_owned(),
        tool_name,
        input: String::new(),
        output: String::new(),
        started_at: Some(now()),
        finished_at: None,
    });
    trace.entries.last_mut().expect("trace entry was inserted")
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

fn bound_text(value: &str) -> String {
    let mut output = String::new();
    append_bounded(&mut output, value);
    output
}
