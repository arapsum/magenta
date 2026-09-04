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

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RegenerateMessageError {
    #[error("the response to regenerate was not found")]
    TargetNotFound { target: MessageId },

    #[error("only assistant responses can be regenerated")]
    TargetNotAssistant { target: MessageId },

    #[error("a response cannot be regenerated while it is still streaming")]
    TargetStillStreaming { target: MessageId },

    #[error("the replacement assistant message ID is already in use")]
    MessageIdAlreadyUsed { message: MessageId },

    #[error("conversation history contains a message from another conversation")]
    HistoryConversationMismatch {
        expected: ConversationId,
        actual: ConversationId,
        message: MessageId,
    },

    #[error("the response has no preceding user message to regenerate from")]
    MissingUserContext { target: MessageId },
}
