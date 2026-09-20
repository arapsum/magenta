use std::sync::Arc;

use magenta_core::{
    ChatProvider, CommandCatalog, ConversationId, ConversationMode, ConversationStore,
    GenerationConfig, GenerationRequest, GenerationStream, Message, MessageId, ProviderId,
};

use crate::trace::traced_generation_stream;
use crate::{RegenerateMessageError, RetryMessageError, apply_provider_prompt, resolve_persisted};

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
    command_catalog: Option<Arc<dyn CommandCatalog>>,
}

impl RegenerateMessage {
    #[must_use]
    pub fn new(provider: Arc<dyn ChatProvider>, store: Arc<dyn ConversationStore>) -> Self {
        Self {
            provider,
            store,
            command_catalog: None,
        }
    }

    #[must_use]
    pub fn with_command_catalog(mut self, catalog: Arc<dyn CommandCatalog>) -> Self {
        self.command_catalog = Some(catalog);
        self
    }

    /// Commits a replacement and loads context before starting the provider.
    ///
    /// # Errors
    /// Returns an error if the target is invalid or persistence fails.
    pub async fn execute(
        &self,
        input: RegenerateMessageInput,
    ) -> Result<PendingRegeneration, RegenerateMessageError> {
        let conversation = self.store.load(input.conversation_id).await?;
        let command_id = self
            .store
            .command_for_response(input.conversation_id, input.target_message_id)
            .await?;
        let resolution = self.resolve_persisted(
            &conversation.conversation.generation,
            &conversation.conversation.mode,
            command_id.as_ref(),
        )?;
        let request_overhead_tokens = resolution
            .as_ref()
            .map_or(0, |resolution| resolution.request_overhead_tokens);
        let prepared = self
            .store
            .begin_regeneration(
                input.conversation_id,
                input.target_message_id,
                request_overhead_tokens,
            )
            .await?;
        let provider_id = prepared.conversation.generation.provider.clone();
        let stream = traced_generation_stream(
            self.store.clone(),
            prepared.assistant_message.id,
            prepared.assistant_message.assistant_trace.clone(),
            self.provider.stream(GenerationRequest {
                generation: prepared.conversation.generation,
                messages: apply_provider_prompt(prepared.context, resolution.as_ref()),
                instructions: resolution.and_then(|resolution| resolution.instructions),
            }),
        );
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
        let command_id = self
            .store
            .command_for_response(input.conversation_id, input.target_message_id)
            .await?;
        let resolution = self.resolve_persisted(
            &input.generation,
            &ConversationMode::Chat,
            command_id.as_ref(),
        )?;
        let request_overhead_tokens = resolution
            .as_ref()
            .map_or(0, |resolution| resolution.request_overhead_tokens);
        let prepared = self
            .store
            .begin_retry(
                input.conversation_id,
                input.target_message_id,
                input.generation,
                request_overhead_tokens,
            )
            .await?;
        let provider_id = prepared.conversation.generation.provider.clone();
        let stream = traced_generation_stream(
            self.store.clone(),
            prepared.assistant_message.id,
            prepared.assistant_message.assistant_trace.clone(),
            self.provider.stream(GenerationRequest {
                generation: prepared.conversation.generation.clone(),
                messages: apply_provider_prompt(prepared.context, resolution.as_ref()),
                instructions: resolution.and_then(|resolution| resolution.instructions),
            }),
        );
        Ok(PendingRetry {
            conversation: prepared.conversation,
            assistant_message: prepared.assistant_message,
            provider_id,
            stream,
            assistant_sequence: prepared.assistant_sequence,
            context_report: prepared.context_report,
        })
    }

    fn resolve_persisted(
        &self,
        generation: &GenerationConfig,
        mode: &ConversationMode,
        command_id: Option<&magenta_core::CommandId>,
    ) -> Result<Option<crate::CommandResolution>, crate::CommandResolutionError> {
        let Some(command_id) = command_id else {
            return Ok(None);
        };
        let catalog = self.command_catalog.as_deref().ok_or_else(|| {
            crate::CommandResolutionError::Unavailable {
                command_id: command_id.clone(),
            }
        })?;
        resolve_persisted(catalog, generation, mode, Some(command_id))
    }
}
