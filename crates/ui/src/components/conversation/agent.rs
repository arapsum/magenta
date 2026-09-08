use futures_util::StreamExt as _;
use magenta_application::PendingAgentGeneration;
use magenta_core::{
    AgentActivity, AgentActivityKind, AgentApprovalDecision, AgentApprovalSubject, AgentRunEvent,
    AgentRunStream, AgentToolOutput, MessageId, ProviderError, ProviderId,
    WorkspaceCommandOutputStream, WorkspaceCommandResult,
};

use super::*;

enum AgentStreamControl {
    Continue,
    Completed(GenerationOutcome),
}

impl ConversationView {
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
                            view.fail_stream(generation, assistant_id, error, window, cx);
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
                let error = ProviderError::new(provider_id, IncompleteGeneration);
                _ = view.update_in(window, |view, window, cx| {
                    view.fail_stream(generation, assistant_id, error, window, cx);
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
                self.push_stream_chunk(generation, assistant_id, &chunk, cx);
            }
            AgentRunEvent::ToolCall(call) => self.push_agent_activity(
                generation,
                assistant_id,
                AgentActivity {
                    kind: AgentActivityKind::ToolCall,
                    call_id: call.id,
                    tool_name: call.name.clone(),
                    status: "requested".to_owned(),
                    summary: call.name,
                    detail: call.arguments,
                },
                cx,
            ),
            AgentRunEvent::ApprovalRequired(approval) => {
                self.apply_approval_event(generation, assistant_id, approval, cx);
            }
            AgentRunEvent::ToolResult(result) => {
                self.apply_tool_result_event(generation, assistant_id, result, cx);
            }
            AgentRunEvent::WorkspaceChange(change) => {
                cx.emit(ConversationViewEvent::WorkspaceChange(change));
            }
            AgentRunEvent::CommandStarted { call_id, command } => {
                self.live_commands.insert(
                    (assistant_id, call_id),
                    LiveCommand {
                        command,
                        stdout: String::new(),
                        stderr: String::new(),
                        result: None,
                    },
                );
                self.list_state.remeasure_items(0..self.messages.len());
                cx.notify();
            }
            AgentRunEvent::CommandOutput {
                call_id,
                stream,
                chunk,
            } => {
                if let Some(command) = self.live_commands.get_mut(&(assistant_id, call_id)) {
                    let output = match stream {
                        WorkspaceCommandOutputStream::Stdout => &mut command.stdout,
                        WorkspaceCommandOutputStream::Stderr => &mut command.stderr,
                    };
                    append_bounded(output, &chunk);
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
        self.push_agent_activity(
            generation,
            assistant_id,
            AgentActivity {
                kind: AgentActivityKind::ApprovalRequested,
                call_id: approval.tool_call_id,
                tool_name: approval.tool_name,
                status: "awaiting-approval".to_owned(),
                summary: approval.reason,
                detail,
            },
            cx,
        );
    }

    fn apply_tool_result_event(
        &mut self,
        generation: u64,
        assistant_id: MessageId,
        result: AgentToolOutput,
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
        self.push_agent_activity(
            generation,
            assistant_id,
            AgentActivity {
                kind: AgentActivityKind::ToolResult,
                call_id: result.call_id,
                tool_name,
                status: if result.is_error {
                    "failed".to_owned()
                } else {
                    "completed".to_owned()
                },
                summary: "Tool result".to_owned(),
                detail: result.output,
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
                    .agent_activities
                    .iter()
                    .rev()
                    .find(|activity| activity.call_id == call_id)
            })
            .map_or_else(
                || "workspace tool".to_owned(),
                |activity| activity.tool_name.clone(),
            )
    }

    pub(super) fn push_agent_activity(
        &mut self,
        generation: u64,
        assistant_id: MessageId,
        activity: AgentActivity,
        cx: &mut Context<'_, Self>,
    ) {
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
        message.message.agent_activities.push(activity);
        self.list_state.remeasure_items(0..self.messages.len());
        cx.notify();
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
