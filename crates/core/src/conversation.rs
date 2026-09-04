use super::{ConversationId, GenerationConfig};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conversation {
    pub id: ConversationId,
    pub title: String,
    pub generation: GenerationConfig,
}
