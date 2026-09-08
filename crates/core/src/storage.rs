//! Durable conversation values and the persistence port. No database types cross this boundary.

use std::{error::Error, future::Future, ops::Range, pin::Pin};

use crate::{
    AgentActivity, AgentActivityRecord, AgentRunId, AttachmentDraft, Conversation, ConversationId,
    ConversationMode, GenerationConfig, Message, MessageId, ProviderId,
};

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
    pub mode: ConversationMode,
    pub workspace_root: Option<std::path::PathBuf>,
    pub provider: ProviderId,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConversationSearchResult {
    pub conversation_id: ConversationId,
    pub message_id: Option<MessageId>,
    pub message_sequence: Option<MessageSequence>,
    pub title: String,
    pub title_highlights: Vec<Range<usize>>,
    pub snippet: String,
    pub snippet_highlights: Vec<Range<usize>>,
    pub updated_at: Timestamp,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredMessage {
    pub message: Message,
    pub sequence: MessageSequence,
    pub created_at: Timestamp,
    pub generation: GenerationConfig,
    pub agent_activities: Vec<AgentActivity>,
    pub omitted_context_messages: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MessagePage {
    pub messages: Vec<StoredMessage>,
    pub older_cursor: Option<MessageSequence>,
    pub has_older: bool,
    pub newer_cursor: Option<MessageSequence>,
    pub has_newer: bool,
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
    pub attachments: Vec<AttachmentDraft>,
    pub generation: GenerationConfig,
    pub mode: ConversationMode,
    pub workspace_root: Option<std::path::PathBuf>,
    pub request_overhead_tokens: u64,
}

pub struct PreparedTurn {
    pub conversation: Conversation,
    pub user_message: Message,
    pub assistant_message: Message,
    pub context: Vec<Message>,
    pub agent_run_id: Option<AgentRunId>,
    pub user_sequence: MessageSequence,
    pub assistant_sequence: MessageSequence,
    pub context_report: crate::ContextBudgetReport,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StorageErrorKind {
    Unavailable,
    InvalidData,
    UnsupportedVersion,
    NotFound,
    Conflict,
    TooManyAttachments,
    AttachmentUnreadable,
    UnsupportedAttachment,
    AnimatedImage,
    AttachmentTooLarge,
    ContextTooLarge,
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
    fn search(&self, query: String, limit: usize) -> StorageFuture<Vec<ConversationSearchResult>>;
    fn load(&self, id: ConversationId) -> StorageFuture<ConversationPage>;
    fn load_around(
        &self,
        id: ConversationId,
        sequence: MessageSequence,
    ) -> StorageFuture<ConversationPage>;
    fn earlier(&self, id: ConversationId, before: MessageSequence) -> StorageFuture<MessagePage>;
    fn later(&self, id: ConversationId, after: MessageSequence) -> StorageFuture<MessagePage>;
    fn begin_turn(&self, input: BeginTurn) -> StorageFuture<PreparedTurn>;
    fn begin_regeneration(
        &self,
        id: ConversationId,
        target: MessageId,
        request_overhead_tokens: u64,
    ) -> StorageFuture<PreparedTurn>;
    fn finalize(&self, message: Message) -> StorageFuture<()>;
    fn delete(&self, id: ConversationId) -> StorageFuture<()>;
    fn rename(&self, id: ConversationId, title: String) -> StorageFuture<()>;
    fn rename_if_current(
        &self,
        id: ConversationId,
        current: String,
        title: String,
    ) -> StorageFuture<bool>;
    fn set_pinned(&self, id: ConversationId, pinned: bool) -> StorageFuture<()>;
    fn append_agent_activity(&self, activity: AgentActivityRecord) -> StorageFuture<()>;
}
