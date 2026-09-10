use std::sync::Arc;

use magenta_core::{
    ChatProvider, ConversationId, ConversationMode, ConversationStore, GenerationConfig,
    GenerationRequest, GenerationStream, Message, MessageId, ProviderId,
};

use crate::{RegenerateMessageError, RetryMessageError};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RegenerateMessageInput {
    pub conversation_id: ConversationId,
    pub target_message_id: MessageId,
}

pub struct PendingRegeneration {
    pub target_message_id: MessageId,
    pub assistant_message: Message,
    pub provider_id: ProviderId,
    pub stream: GenerationStream,
    pub assistant_sequence: magenta_core::MessageSequence,
    pub context_report: magenta_core::ContextBudgetReport,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RetryMessageInput {
    pub conversation_id: ConversationId,
    pub target_message_id: MessageId,
    pub generation: GenerationConfig,
}

pub struct PendingRetry {
    pub conversation: magenta_core::Conversation,
    pub assistant_message: Message,
    pub provider_id: ProviderId,
    pub stream: GenerationStream,
    pub assistant_sequence: magenta_core::MessageSequence,
    pub context_report: magenta_core::ContextBudgetReport,
}

#[derive(Clone)]
pub struct RegenerateMessage {
    provider: Arc<dyn ChatProvider>,
    store: Arc<dyn ConversationStore>,
}

impl RegenerateMessage {
    #[must_use]
    pub fn new(provider: Arc<dyn ChatProvider>, store: Arc<dyn ConversationStore>) -> Self {
        Self { provider, store }
    }

    /// Commits a replacement and loads context before starting the provider.
    ///
    /// # Errors
    /// Returns an error if the target is invalid or persistence fails.
    pub async fn execute(
        &self,
        input: RegenerateMessageInput,
    ) -> Result<PendingRegeneration, RegenerateMessageError> {
        let prepared = self
            .store
            .begin_regeneration(input.conversation_id, input.target_message_id, 0)
            .await?;
        let provider_id = prepared.conversation.generation.provider.clone();
        let stream = self.provider.stream(GenerationRequest {
            generation: prepared.conversation.generation,
            messages: prepared.context,
        });
        Ok(PendingRegeneration {
            target_message_id: input.target_message_id,
            assistant_message: prepared.assistant_message,
            provider_id,
            stream,
            assistant_sequence: prepared.assistant_sequence,
            context_report: prepared.context_report,
        })
    }

    /// Starts a new assistant attempt without replacing the failed transcript
    /// entry. Failed output is excluded by the storage context query.
    ///
    /// # Errors
    ///
    /// Returns an error when the target is invalid, storage is unavailable, or
    /// the conversation is an agent run that needs a workspace continuation.
    pub async fn retry(&self, input: RetryMessageInput) -> Result<PendingRetry, RetryMessageError> {
        if self
            .store
            .load(input.conversation_id)
            .await?
            .conversation
            .mode
            == ConversationMode::Agent
        {
            return Err(RetryMessageError::AgentContinuation);
        }
        let prepared = self
            .store
            .begin_retry(
                input.conversation_id,
                input.target_message_id,
                input.generation,
                0,
            )
            .await?;
        let provider_id = prepared.conversation.generation.provider.clone();
        let stream = self.provider.stream(GenerationRequest {
            generation: prepared.conversation.generation.clone(),
            messages: prepared.context,
        });
        Ok(PendingRetry {
            conversation: prepared.conversation,
            assistant_message: prepared.assistant_message,
            provider_id,
            stream,
            assistant_sequence: prepared.assistant_sequence,
            context_report: prepared.context_report,
        })
    }
}
