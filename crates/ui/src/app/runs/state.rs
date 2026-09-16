use std::collections::HashMap;

use magenta_application::AgentApprovalController;
use magenta_core::{AgentApprovalRequest, Conversation, Message, MessageId, MessageStatus};

use crate::components::conversation::{
    GenerationProgress, LiveCommand, LiveRunMessage, LiveRunSnapshot,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunSaveState {
    Pending,
    Saving,
    Saved,
    Failed,
}

pub struct ResponseRun {
    pub(crate) conversation: Conversation,
    pub(crate) user: Option<LiveRunMessage>,
    pub(crate) assistant: LiveRunMessage,
    pub(crate) progress: GenerationProgress,
    pub(crate) streaming: bool,
    pub(crate) controller: Option<AgentApprovalController>,
    pub(crate) pending_approval: Option<(MessageId, AgentApprovalRequest)>,
    pub(crate) live_commands: HashMap<(MessageId, String), LiveCommand>,
    pub(crate) save_state: RunSaveState,
}

impl ResponseRun {
    pub const fn assistant_id(&self) -> MessageId {
        self.assistant.message.id
    }

    pub const fn is_active(&self) -> bool {
        self.streaming
    }

    pub const fn is_awaiting_approval(&self) -> bool {
        self.pending_approval.is_some()
    }

    pub const fn is_unsaved(&self) -> bool {
        matches!(self.save_state, RunSaveState::Failed)
    }

    pub(crate) fn snapshot(&self) -> LiveRunSnapshot {
        let mut messages = Vec::with_capacity(2);
        if let Some(user) = &self.user {
            messages.push(user.clone());
        }
        messages.push(self.assistant.clone());
        LiveRunSnapshot {
            conversation: self.conversation.clone(),
            messages,
            streaming_message: self.streaming.then_some(self.assistant_id()),
            generation_progress: self.streaming.then(|| self.progress.clone()),
            agent_controller: self.controller.clone(),
            pending_agent_approval: self.pending_approval.clone(),
            live_commands: self.live_commands.clone(),
        }
    }

    pub(crate) fn terminal_message(&self) -> Message {
        self.assistant.message.clone()
    }

    pub(crate) fn mark_terminal(
        &mut self,
        status: MessageStatus,
        outcome: Option<magenta_core::GenerationOutcome>,
        failure: Option<magenta_core::MessageFailure>,
    ) {
        self.streaming = false;
        self.assistant.message.status = status;
        self.assistant.message.generation_outcome = outcome;
        self.assistant.message.failure = failure;
        self.progress.final_answer_started = false;
    }
}
