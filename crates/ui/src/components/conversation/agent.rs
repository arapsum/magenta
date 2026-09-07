use futures_util::StreamExt as _;
use magenta_application::PendingAgentGeneration;
use magenta_core::{
    AgentActivity, AgentActivityKind, AgentApprovalDecision, AgentRunEvent, AgentRunStream,
    MessageId, ProviderError, ProviderId,
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
        self.list_state.splice(old_count..old_count, 2);
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
                self.pending_agent_approval = Some((assistant_id, approval.clone()));
                self.push_agent_activity(
                    generation,
                    assistant_id,
                    AgentActivity {
                        kind: AgentActivityKind::ApprovalRequested,
                        call_id: approval.tool_call_id.clone(),
                        tool_name: approval.tool_name.clone(),
                        status: "awaiting-approval".to_owned(),
                        summary: approval.reason.clone(),
                        detail: approval.diff.unwrap_or_default(),
                    },
                    cx,
                );
            }
            AgentRunEvent::ToolResult(result) => {
                let tool_name = self.tool_name_for_result(assistant_id, &result.call_id);
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
            AgentRunEvent::WorkspaceChange(change) => {
                cx.emit(ConversationViewEvent::WorkspaceChange(change));
            }
            AgentRunEvent::Completed(outcome) => return AgentStreamControl::Completed(outcome),
        }
        AgentStreamControl::Continue
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
