use std::sync::Arc;

use magenta_core::{
    Attachment, BeginTurn, ChatProvider, Conversation, ConversationId, ConversationStore,
    GenerationConfig, GenerationRequest, GenerationStream, Message,
};

use crate::SendMessageError;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SendTarget {
    New,
    Existing(ConversationId),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SendMessageInput {
    pub target: SendTarget,
    pub prompt: String,
    pub attachments: Vec<Attachment>,
    pub generation: GenerationConfig,
}

pub struct PendingGeneration {
    pub conversation: Conversation,
    pub user_message: Message,
    pub assistant_message: Message,
    pub stream: GenerationStream,
}

#[derive(Clone)]
pub struct SendMessage {
    provider: Arc<dyn ChatProvider>,
    store: Arc<dyn ConversationStore>,
}

impl SendMessage {
    #[must_use]
    pub fn new(provider: Arc<dyn ChatProvider>, store: Arc<dyn ConversationStore>) -> Self {
        Self { provider, store }
    }

    /// Commits the turn before starting the provider.
    ///
    /// # Errors
    /// Returns an error if the prompt is empty or persistence fails.
    pub async fn execute(
        &self,
        input: SendMessageInput,
    ) -> Result<PendingGeneration, SendMessageError> {
        let prompt = input.prompt.trim().to_owned();
        if prompt.is_empty() {
            return Err(SendMessageError::EmptyPrompt);
        }
        let prepared = self
            .store
            .begin_turn(BeginTurn {
                conversation_id: match input.target {
                    SendTarget::New => None,
                    SendTarget::Existing(id) => Some(id),
                },
                title: title_from_prompt(&prompt),
                prompt,
                attachments: input.attachments,
                generation: input.generation,
            })
            .await?;
        let stream = self.provider.stream(GenerationRequest {
            generation: prepared.conversation.generation.clone(),
            messages: prepared.context,
        });
        Ok(PendingGeneration {
            conversation: prepared.conversation,
            user_message: prepared.user_message,
            assistant_message: prepared.assistant_message,
            stream,
        })
    }
}

fn title_from_prompt(prompt: &str) -> String {
    let title = prompt
        .lines()
        .next()
        .unwrap_or_default()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if title.chars().count() > 46 {
        format!("{}...", title.chars().take(43).collect::<String>())
    } else if title.is_empty() {
        "New conversation".to_owned()
    } else {
        title
    }
}
