use std::sync::Arc;

use magenta_core::{
    ChatProvider, ConversationId, ConversationStore, GenerationRequest, GenerationStream, Message,
    MessageId, ProviderId,
};

use crate::RegenerateMessageError;

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
            .begin_regeneration(input.conversation_id, input.target_message_id)
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
        })
    }
}
