use magenta_core::{ConversationId, MessageId};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SendMessageError {
    #[error("the prompt cannot be empty")]
    EmptyPrompt,

    #[error("the user and assistant message IDs must be different")]
    DuplicateMessageIds,

    #[error("a new message ID is already present in the conversation history")]
    MessageIdAlreadyUsed,

    #[error("conversation history contains a message from another conversation")]
    HistoryConversationMismatch {
        expected: ConversationId,
        actual: ConversationId,
        message: MessageId,
    },
}
