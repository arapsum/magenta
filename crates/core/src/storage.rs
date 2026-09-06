//! Durable conversation values and the persistence port. No database types cross this boundary.

use std::{error::Error, future::Future, pin::Pin};

use crate::{Attachment, Conversation, ConversationId, GenerationConfig, Message, MessageId};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Timestamp(pub i64);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct MessageSequence(pub i64);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConversationSummary {
    pub id: ConversationId,
    pub title: String,
    pub preview: String,
    pub pinned: bool,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredMessage {
    pub message: Message,
    pub sequence: MessageSequence,
    pub created_at: Timestamp,
    pub generation: GenerationConfig,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MessagePage {
    pub messages: Vec<StoredMessage>,
    pub older_cursor: Option<MessageSequence>,
    pub has_older: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConversationPage {
    pub conversation: Conversation,
    pub page: MessagePage,
}

#[derive(Clone, Debug)]
pub struct BeginTurn {
    pub conversation_id: Option<ConversationId>,
    pub title: String,
    pub prompt: String,
    pub attachments: Vec<Attachment>,
    pub generation: GenerationConfig,
}

pub struct PreparedTurn {
    pub conversation: Conversation,
    pub user_message: Message,
    pub assistant_message: Message,
    pub context: Vec<Message>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StorageErrorKind {
    Unavailable,
    InvalidData,
    UnsupportedVersion,
    NotFound,
    Conflict,
}

#[derive(Debug, thiserror::Error)]
#[error("conversation storage operation failed ({kind:?})")]
pub struct StorageError {
    pub kind: StorageErrorKind,
    #[source]
    pub source: Box<dyn Error + Send + Sync>,
}

impl StorageError {
    #[must_use]
    pub fn new(kind: StorageErrorKind, source: impl Error + Send + Sync + 'static) -> Self {
        Self {
            kind,
            source: Box::new(source),
        }
    }
}

pub type StorageFuture<T> = Pin<Box<dyn Future<Output = Result<T, StorageError>> + Send + 'static>>;

/// Writes must be atomic. Finalization only changes the addressed streaming message.
pub trait ConversationStore: Send + Sync {
    fn initialize(&self) -> StorageFuture<()>;
    fn summaries(&self) -> StorageFuture<Vec<ConversationSummary>>;
    fn load(&self, id: ConversationId) -> StorageFuture<ConversationPage>;
    fn earlier(&self, id: ConversationId, before: MessageSequence) -> StorageFuture<MessagePage>;
    fn begin_turn(&self, input: BeginTurn) -> StorageFuture<PreparedTurn>;
    fn begin_regeneration(
        &self,
        id: ConversationId,
        target: MessageId,
    ) -> StorageFuture<PreparedTurn>;
    fn finalize(&self, message: Message) -> StorageFuture<()>;
    fn delete(&self, id: ConversationId) -> StorageFuture<()>;
    fn rename(&self, id: ConversationId, title: String) -> StorageFuture<()>;
    fn set_pinned(&self, id: ConversationId, pinned: bool) -> StorageFuture<()>;
}
