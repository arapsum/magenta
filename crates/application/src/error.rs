use magenta_core::StorageError;

#[derive(Debug, thiserror::Error)]
pub enum SendMessageError {
    #[error("the prompt cannot be empty")]
    EmptyPrompt,
    #[error("could not persist the message turn")]
    Storage(#[from] StorageError),
}

#[derive(Debug, thiserror::Error)]
pub enum RegenerateMessageError {
    #[error("could not prepare a persisted response replacement")]
    Storage(#[from] StorageError),
}
