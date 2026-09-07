use super::{ConversationId, ConversationMode, GenerationConfig};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conversation {
    pub id: ConversationId,
    pub title: String,
    pub generation: GenerationConfig,
    pub mode: ConversationMode,
    pub workspace_root: Option<std::path::PathBuf>,
}
