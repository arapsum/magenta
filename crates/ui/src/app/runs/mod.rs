mod persistence;
mod state;
mod stream;

use std::collections::HashMap;

use gpui_kit::Task;
use magenta_core::{
    ConversationId, ConversationPage, Message, MessageId, MessageSequence, MessageStatus,
    StoredMessage, Timestamp,
};

use crate::components::conversation::{LiveRunMessage, LiveRunSnapshot};

pub use state::{ResponseRun, RunSaveState};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunIndicator {
    Running,
    AwaitingApproval,
    Unsaved,
}

#[derive(Default)]
pub struct ResponseRunCoordinator {
    pub(crate) runs: HashMap<MessageId, ResponseRun>,
    pub(crate) active_by_conversation: HashMap<ConversationId, MessageId>,
    pub(crate) stream_tasks: HashMap<MessageId, Task<()>>,
    pub(crate) save_tasks: HashMap<MessageId, Task<()>>,
    pub(crate) control_tasks: HashMap<MessageId, Task<()>>,
}

impl ResponseRunCoordinator {
    pub(crate) fn run_for_conversation(
        &self,
        conversation_id: ConversationId,
    ) -> Option<&ResponseRun> {
        let message_id = self.active_by_conversation.get(&conversation_id)?;
        self.runs.get(message_id)
    }

    pub(crate) fn run_for_message(&self, message_id: MessageId) -> Option<&ResponseRun> {
        self.runs.get(&message_id)
    }

    pub(crate) fn has_active_for(&self, conversation_id: Option<ConversationId>) -> bool {
        conversation_id
            .and_then(|id| self.run_for_conversation(id))
            .is_some_and(ResponseRun::is_active)
    }

    pub(crate) fn has_unsaved_for(&self, conversation_id: Option<ConversationId>) -> bool {
        conversation_id
            .and_then(|id| self.run_for_conversation(id))
            .is_some_and(ResponseRun::is_unsaved)
    }

    pub(crate) fn has_unfinalized_for(&self, conversation_id: Option<ConversationId>) -> bool {
        conversation_id
            .and_then(|id| self.run_for_conversation(id))
            .is_some_and(|run| run.save_state != RunSaveState::Saved)
    }

    pub(crate) fn blocks_deletion(&self, conversation_id: ConversationId) -> bool {
        self.run_for_conversation(conversation_id)
            .is_some_and(|run| {
                run.is_active()
                    || run.is_unsaved()
                    || run.controller.as_ref().is_some_and(
                        magenta_application::AgentApprovalController::has_pending_workspace_review,
                    )
            })
    }

    pub(crate) fn indicator(&self, conversation_id: ConversationId) -> Option<RunIndicator> {
        let run = self.run_for_conversation(conversation_id)?;
        if run.is_unsaved() {
            Some(RunIndicator::Unsaved)
        } else if run.is_awaiting_approval() {
            Some(RunIndicator::AwaitingApproval)
        } else if run.is_active() {
            Some(RunIndicator::Running)
        } else {
            None
        }
    }

    pub(crate) fn snapshot_for_conversation(
        &self,
        conversation_id: ConversationId,
    ) -> Option<LiveRunSnapshot> {
        self.run_for_conversation(conversation_id)
            .map(ResponseRun::snapshot)
    }

    pub(crate) fn overlay_page(&self, mut page: ConversationPage) -> ConversationPage {
        let Some(run) = self.run_for_conversation(page.conversation.id) else {
            return page;
        };

        for live in run.snapshot().messages {
            overlay_message(&mut page.page.messages, live);
        }
        page.page.messages.sort_by_key(|message| message.sequence);
        page.page.older_cursor = page.page.messages.first().map(|message| message.sequence);
        page.page.newer_cursor = page.page.messages.last().map(|message| message.sequence);
        page
    }

    pub(crate) fn message_for_save(&self, message_id: MessageId) -> Option<Message> {
        self.runs
            .get(&message_id)
            .map(ResponseRun::terminal_message)
    }

    pub(crate) fn mark_saved(&mut self, message_id: MessageId) -> bool {
        let review_is_resolving = self.control_tasks.contains_key(&message_id);
        let Some(run) = self.runs.get_mut(&message_id) else {
            return false;
        };
        if review_is_resolving
            || run.controller.as_ref().is_some_and(
                magenta_application::AgentApprovalController::has_pending_workspace_review,
            )
        {
            run.save_state = RunSaveState::Saved;
            self.stream_tasks.remove(&message_id);
            return false;
        }
        let run = self.runs.remove(&message_id).expect("run was present");
        self.active_by_conversation.remove(&run.conversation.id);
        self.stream_tasks.remove(&message_id);
        true
    }

    pub(crate) fn mark_save_failed(&mut self, message_id: MessageId) {
        if let Some(run) = self.runs.get_mut(&message_id) {
            run.save_state = RunSaveState::Failed;
        }
    }

    pub(crate) fn mark_saving(&mut self, message_id: MessageId) {
        if let Some(run) = self.runs.get_mut(&message_id) {
            run.save_state = RunSaveState::Saving;
        }
    }

    pub(crate) fn mark_pending_save(&mut self, message_id: MessageId) {
        if let Some(run) = self.runs.get_mut(&message_id) {
            run.save_state = RunSaveState::Pending;
        }
    }

    pub(crate) fn remove_run(&mut self, message_id: MessageId) {
        if let Some(run) = self.runs.remove(&message_id) {
            self.active_by_conversation.remove(&run.conversation.id);
        }
        self.stream_tasks.remove(&message_id);
        self.save_tasks.remove(&message_id);
        self.control_tasks.remove(&message_id);
    }

    pub(crate) fn stop_all_active(&mut self) {
        let active = self
            .runs
            .values()
            .filter(|run| run.is_active())
            .map(state::ResponseRun::assistant_id)
            .collect::<Vec<_>>();
        for message_id in active {
            let _ = self.stop(message_id);
        }
    }

    pub(crate) fn shutdown_messages(&self) -> Vec<Message> {
        self.runs
            .iter()
            .filter(|(message_id, run)| {
                !run.is_active()
                    && !self.save_tasks.contains_key(message_id)
                    && matches!(run.save_state, RunSaveState::Pending | RunSaveState::Failed)
            })
            .map(|(_, run)| run.terminal_message())
            .collect()
    }

    pub(crate) fn take_save_tasks(&mut self) -> Vec<Task<()>> {
        self.save_tasks.drain().map(|(_, task)| task).collect()
    }

    pub(crate) fn take_control_tasks(&mut self) -> Vec<Task<()>> {
        self.control_tasks.drain().map(|(_, task)| task).collect()
    }

    pub(crate) fn complete(
        &mut self,
        message_id: MessageId,
        outcome: magenta_core::GenerationOutcome,
    ) -> Option<Message> {
        let run = self.runs.get_mut(&message_id)?;
        let elapsed = duration_millis(run.progress.started_at.elapsed());
        mark_trace_terminal(
            &mut run.assistant.message,
            magenta_core::AssistantTraceStatus::Completed,
            elapsed,
        );
        run.mark_terminal(MessageStatus::Complete, Some(outcome), None);
        Some(run.terminal_message())
    }

    pub(crate) fn fail(
        &mut self,
        message_id: MessageId,
        error: &magenta_core::ProviderError,
    ) -> Option<Message> {
        let run = self.runs.get_mut(&message_id)?;
        let elapsed = duration_millis(run.progress.started_at.elapsed());
        mark_trace_terminal(
            &mut run.assistant.message,
            magenta_core::AssistantTraceStatus::Failed,
            elapsed,
        );
        run.mark_terminal(
            MessageStatus::Failed,
            None,
            Some(magenta_core::MessageFailure::from_provider_error(error)),
        );
        Some(run.terminal_message())
    }

    pub(crate) fn stop(&mut self, message_id: MessageId) -> Option<Message> {
        self.stream_tasks.remove(&message_id);
        let run = self.runs.get_mut(&message_id)?;
        let elapsed = duration_millis(run.progress.started_at.elapsed());
        mark_trace_terminal(
            &mut run.assistant.message,
            magenta_core::AssistantTraceStatus::Stopped,
            elapsed,
        );
        for command in run.live_commands.values_mut() {
            if command.result.is_none() {
                command.result = Some(magenta_core::WorkspaceCommandResult {
                    status: magenta_core::WorkspaceCommandStatus::Cancelled,
                    exit_code: None,
                    duration_ms: 0,
                    stdout: command.stdout.clone(),
                    stderr: command.stderr.clone(),
                    truncated: false,
                });
            }
        }
        run.pending_approval = None;
        run.mark_terminal(MessageStatus::Stopped, None, None);
        Some(run.terminal_message())
    }

    pub(crate) fn decide_agent_approval(
        &mut self,
        message_id: MessageId,
        decision: magenta_core::AgentApprovalDecision,
    ) -> bool {
        let Some(run) = self.runs.get_mut(&message_id) else {
            return false;
        };
        let Some((_, approval)) = run.pending_approval.take() else {
            return false;
        };
        run.controller
            .as_ref()
            .is_some_and(|controller| controller.decide(approval.request_id, decision))
    }

    pub(crate) fn controller_for(
        &self,
        message_id: MessageId,
    ) -> Option<magenta_application::AgentApprovalController> {
        self.runs
            .get(&message_id)
            .and_then(|run| run.controller.clone())
    }

    pub(crate) fn set_controller(
        &mut self,
        message_id: MessageId,
        controller: Option<magenta_application::AgentApprovalController>,
    ) {
        if let Some(run) = self.runs.get_mut(&message_id) {
            run.controller = controller;
        }
    }

    pub(crate) fn remove_after_review(&mut self, message_id: MessageId) {
        if self
            .runs
            .get(&message_id)
            .is_some_and(|run| matches!(run.save_state, RunSaveState::Saved))
        {
            self.remove_run(message_id);
        }
    }

    pub(crate) fn insert_terminal(
        &mut self,
        message: Message,
        conversation: magenta_core::Conversation,
    ) {
        let message_id = message.id;
        let progress = crate::components::conversation::GenerationProgress::new(
            message_id,
            conversation.generation.provider.clone(),
            Some(conversation.generation.clone()),
        );
        self.active_by_conversation
            .insert(conversation.id, message_id);
        let generation = conversation.generation.clone();
        self.runs.insert(
            message_id,
            ResponseRun {
                conversation,
                user: None,
                assistant: crate::components::conversation::LiveRunMessage {
                    message,
                    sequence: None,
                    created_at: now(),
                    generation,
                    omitted_context_messages: 0,
                    replaces: None,
                },
                progress,
                streaming: false,
                controller: None,
                pending_approval: None,
                live_commands: HashMap::new(),
                save_state: RunSaveState::Failed,
            },
        );
    }
}

fn overlay_message(messages: &mut Vec<StoredMessage>, live: LiveRunMessage) {
    let index = live
        .replaces
        .and_then(|id| messages.iter().position(|item| item.message.id == id))
        .or_else(|| {
            messages
                .iter()
                .position(|item| item.message.id == live.message.id)
        });

    if let Some(index) = index {
        let item = &mut messages[index];
        item.message = live.message;
        item.generation = live.generation;
        if let Some(sequence) = live.sequence {
            item.sequence = sequence;
        }
        item.created_at = live.created_at;
        item.omitted_context_messages = live.omitted_context_messages;
        return;
    }

    let sequence = live.sequence.unwrap_or_else(|| {
        messages.last().map_or(MessageSequence(0), |item| {
            MessageSequence(item.sequence.0.saturating_add(1))
        })
    });
    messages.push(StoredMessage {
        message: live.message,
        sequence,
        created_at: live.created_at,
        generation: live.generation,
        omitted_context_messages: live.omitted_context_messages,
    });
}

pub fn now() -> Timestamp {
    Timestamp(chrono::Local::now().timestamp_millis())
}

pub fn mark_trace_terminal(
    message: &mut Message,
    status: magenta_core::AssistantTraceStatus,
    elapsed_ms: u64,
) {
    message.assistant_trace.thinking_duration_ms = Some(elapsed_ms);
    let finished_at = now();
    for entry in &mut message.assistant_trace.entries {
        if matches!(
            entry.status,
            magenta_core::AssistantTraceStatus::Streaming
                | magenta_core::AssistantTraceStatus::Requested
                | magenta_core::AssistantTraceStatus::Running
                | magenta_core::AssistantTraceStatus::AwaitingApproval
        ) {
            entry.status = status;
            entry.finished_at = Some(finished_at);
        }
    }
}

pub fn duration_millis(duration: std::time::Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}
