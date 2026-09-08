use magenta_core::{ProviderError, StorageError};

#[derive(Debug, thiserror::Error)]
pub enum SendMessageError {
    #[error("the prompt cannot be empty")]
    EmptyPrompt,
    #[error("could not persist the message turn")]
    Storage(#[from] StorageError),
    #[error("the selected workspace is unavailable")]
    WorkspaceUnavailable,
}

#[derive(Debug, thiserror::Error)]
pub enum RegenerateMessageError {
    #[error("could not prepare a persisted response replacement")]
    Storage(#[from] StorageError),
}

#[derive(Debug, thiserror::Error)]
pub enum TitleConversationError {
    #[error("the title generation request failed")]
    Provider(#[from] ProviderError),
    #[error("the title generation response was incomplete")]
    Incomplete,
    #[error("the generated title was empty")]
    Empty,
    #[error("the generated title could not be persisted")]
    Storage(#[from] StorageError),
}
