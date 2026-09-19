//! Local Turso implementation of Magenta's primary application store.

use std::{
    future::Future,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use crate::{attachments, failure, invalid, now, records, unavailable};
use magenta_core::{
    AgentRunId, AssistantTrace, AssistantTraceEntry, AssistantTraceKind, AssistantTraceStatus,
    Attachment, BeginTurn, ContextBudgetReport, Conversation, ConversationId, ConversationMode,
    ConversationPage, ConversationSearchResult, ConversationStore, ConversationSummary,
    GenerationConfig, Message, MessageId, MessagePage, MessageRole, MessageSequence, MessageStatus,
    PreparedTurn, Project, ProjectStore, StorageError, StorageErrorKind, StorageFuture,
    StoredMessage, Timestamp, select_context,
};
use rusqlite::OptionalExtension;
use turso::{
    Connection, Row, params,
    transaction::{Transaction, TransactionBehavior},
};

mod conversation;
mod migration;
mod project;
mod reading;
mod trace;
mod turns;

type Result<T> = std::result::Result<T, StorageError>;
const SCHEMA_VERSION: i64 = 2;
const PAGE_SIZE: usize = 50;
const BUSY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

const SCHEMA: &str = r"
CREATE TABLE IF NOT EXISTS _magenta_schema(component TEXT PRIMARY KEY, version INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS conversations (
 id INTEGER PRIMARY KEY AUTOINCREMENT, title TEXT NOT NULL, generation TEXT NOT NULL,
 mode TEXT NOT NULL, workspace_root BLOB, pinned INTEGER NOT NULL DEFAULT 0,
 created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL
);
-- Turso 0.4.4 can corrupt a secondary index when an indexed recency value is
-- updated in the same transaction. These datasets are small and ordering does
-- not require an index, so remove indexes over mutable recency columns.
DROP INDEX IF EXISTS conversation_recency;
CREATE TABLE IF NOT EXISTS messages (
 id INTEGER PRIMARY KEY AUTOINCREMENT, conversation_id INTEGER NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
 sequence INTEGER NOT NULL, role TEXT NOT NULL, content TEXT NOT NULL, status TEXT NOT NULL,
 generation TEXT NOT NULL, outcome TEXT, failure TEXT, omitted_context_messages INTEGER NOT NULL DEFAULT 0,
 thinking_duration_ms INTEGER,
 created_at INTEGER NOT NULL, UNIQUE(conversation_id, sequence)
);
CREATE UNIQUE INDEX IF NOT EXISTS one_stream_per_conversation ON messages(conversation_id) WHERE status = 'streaming';
CREATE INDEX IF NOT EXISTS message_conversation_order ON messages(conversation_id, sequence);
CREATE TABLE IF NOT EXISTS projects (
 root BLOB PRIMARY KEY, name TEXT NOT NULL, added_at INTEGER NOT NULL, last_opened_at INTEGER NOT NULL
);
DROP INDEX IF EXISTS project_recency;
CREATE TABLE IF NOT EXISTS agent_runs (
 id INTEGER PRIMARY KEY AUTOINCREMENT, conversation_id INTEGER NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
 assistant_message_id INTEGER NOT NULL UNIQUE REFERENCES messages(id) ON DELETE CASCADE,
 status TEXT NOT NULL, started_at INTEGER NOT NULL, finished_at INTEGER
);
CREATE TABLE IF NOT EXISTS assistant_traces (
 id INTEGER PRIMARY KEY AUTOINCREMENT,
 assistant_message_id INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
 trace_key TEXT NOT NULL, sequence INTEGER NOT NULL, kind TEXT NOT NULL, status TEXT NOT NULL,
 title TEXT NOT NULL, tool_name TEXT, input TEXT NOT NULL, output TEXT NOT NULL,
 started_at INTEGER, finished_at INTEGER,
 UNIQUE(assistant_message_id, trace_key),
 UNIQUE(assistant_message_id, sequence)
);
CREATE INDEX IF NOT EXISTS assistant_trace_order
 ON assistant_traces(assistant_message_id, sequence);
CREATE TABLE IF NOT EXISTS attachments (
 message_id INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE, position INTEGER NOT NULL,
 name TEXT NOT NULL, source_path BLOB NOT NULL, mime_type TEXT NOT NULL, byte_size INTEGER NOT NULL,
 managed INTEGER NOT NULL, PRIMARY KEY(message_id, position)
);
";

#[derive(Clone)]
pub struct TursoAppStore {
    path: Arc<PathBuf>,
    attachments_path: Arc<PathBuf>,
    initialized: Arc<AtomicBool>,
}

impl TursoAppStore {
    #[must_use]
    pub fn new(path: PathBuf) -> Self {
        Self {
            attachments_path: Arc::new(super::attachment_directory(&path)),
            path: Arc::new(path),
            initialized: Arc::new(AtomicBool::new(false)),
        }
    }

    async fn connect_path(path: &std::path::Path) -> Result<Connection> {
        let database = turso::Builder::new_local(&path.to_string_lossy())
            .build()
            .await
            .map_err(db)?;

        let connection = database.connect().map_err(db)?;

        connection.busy_timeout(BUSY_TIMEOUT).map_err(db)?;

        connection
            .execute("PRAGMA foreign_keys = ON", ())
            .await
            .map_err(db)?;
        Ok(connection)
    }

    fn run<T, F, Fut>(&self, operation: F) -> StorageFuture<T>
    where
        T: Send + 'static,
        F: FnOnce(Connection) -> Fut + Send + 'static,
        Fut: Future<Output = Result<T>> + Send + 'static,
    {
        let path = Arc::clone(&self.path);
        let initialized = Arc::clone(&self.initialized);

        Box::pin(async move {
            if !initialized.load(Ordering::Acquire) {
                return Err(super::failure(
                    StorageErrorKind::Unavailable,
                    "storage has not been initialized",
                ));
            }
            operation(Self::connect_path(&path).await?).await
        })
    }
}

struct LegacyTrace {
    call_id: String,
    tool_name: String,
    title: String,
    input: String,
    output: String,
    status: AssistantTraceStatus,
    started_at: i64,
    finished_at: Option<i64>,
}

async fn schema_version(connection: &Connection) -> Result<i64> {
    let mut statement = connection
        .prepare("SELECT version FROM _magenta_schema WHERE component='app'")
        .await
        .map_err(db)?;
    let mut rows = statement.query(()).await.map_err(db)?;

    Ok(rows
        .next()
        .await
        .map_err(db)?
        .map(|row| row.get::<i64>(0))
        .transpose()
        .map_err(db)?
        .unwrap_or(0))
}

async fn scalar_i64(
    connection: &Connection,
    sql: &str,
    parameters: impl turso::params::IntoParams,
) -> Result<i64> {
    let mut statement = connection.prepare(sql).await.map_err(db)?;
    let mut rows = statement.query(parameters).await.map_err(db)?;

    let row =
        rows.next().await.map_err(db)?.ok_or_else(|| {
            super::failure(StorageErrorKind::InvalidData, "query returned no value")
        })?;
    row.get(0).map_err(db)
}

fn db(error: turso::Error) -> StorageError {
    StorageError::new(StorageErrorKind::Unavailable, error)
}

fn as_i64(value: u64) -> Result<i64> {
    i64::try_from(value).map_err(super::invalid)
}
fn as_u64(value: i64) -> Result<u64> {
    u64::try_from(value).map_err(super::invalid)
}
const fn mode_name(mode: &ConversationMode) -> &'static str {
    match mode {
        ConversationMode::Chat => "chat",
        ConversationMode::Agent => "agent",
    }
}
fn parse_mode(value: &str) -> Result<ConversationMode> {
    match value {
        "chat" => Ok(ConversationMode::Chat),
        "agent" => Ok(ConversationMode::Agent),
        _ => Err(super::failure(
            StorageErrorKind::InvalidData,
            "unknown conversation mode",
        )),
    }
}
fn parse_role(value: &str) -> Result<MessageRole> {
    match value {
        "user" => Ok(MessageRole::User),
        "assistant" => Ok(MessageRole::Assistant),
        _ => Err(super::failure(
            StorageErrorKind::InvalidData,
            "unknown message role",
        )),
    }
}
const fn status_name(value: MessageStatus) -> &'static str {
    match value {
        MessageStatus::Complete => "complete",
        MessageStatus::Streaming => "streaming",
        MessageStatus::Stopped => "stopped",
        MessageStatus::Failed => "failed",
    }
}
fn parse_status(value: &str) -> Result<MessageStatus> {
    match value {
        "complete" => Ok(MessageStatus::Complete),
        "streaming" => Ok(MessageStatus::Streaming),
        "stopped" => Ok(MessageStatus::Stopped),
        "failed" => Ok(MessageStatus::Failed),
        _ => Err(super::failure(
            StorageErrorKind::InvalidData,
            "unknown message status",
        )),
    }
}
const fn run_status(value: MessageStatus) -> &'static str {
    match value {
        MessageStatus::Complete => "completed",
        MessageStatus::Stopped => "stopped",
        MessageStatus::Failed => "failed",
        MessageStatus::Streaming => "running",
    }
}
const fn trace_kind(value: AssistantTraceKind) -> &'static str {
    match value {
        AssistantTraceKind::ReasoningSummary => "reasoning_summary",
        AssistantTraceKind::Tool => "tool",
    }
}
fn parse_trace_kind(value: &str) -> Result<AssistantTraceKind> {
    match value {
        "reasoning_summary" => Ok(AssistantTraceKind::ReasoningSummary),
        "tool" => Ok(AssistantTraceKind::Tool),
        _ => Err(super::failure(
            StorageErrorKind::InvalidData,
            "unknown assistant trace kind",
        )),
    }
}
const fn trace_status(value: AssistantTraceStatus) -> &'static str {
    match value {
        AssistantTraceStatus::Streaming => "streaming",
        AssistantTraceStatus::Requested => "requested",
        AssistantTraceStatus::Running => "running",
        AssistantTraceStatus::AwaitingApproval => "awaiting_approval",
        AssistantTraceStatus::Completed => "completed",
        AssistantTraceStatus::Rejected => "rejected",
        AssistantTraceStatus::Failed => "failed",
        AssistantTraceStatus::Stopped => "stopped",
    }
}
fn parse_trace_status(value: &str) -> Result<AssistantTraceStatus> {
    match value {
        "streaming" => Ok(AssistantTraceStatus::Streaming),
        "requested" => Ok(AssistantTraceStatus::Requested),
        "running" => Ok(AssistantTraceStatus::Running),
        "awaiting_approval" => Ok(AssistantTraceStatus::AwaitingApproval),
        "completed" => Ok(AssistantTraceStatus::Completed),
        "rejected" => Ok(AssistantTraceStatus::Rejected),
        "failed" => Ok(AssistantTraceStatus::Failed),
        "stopped" => Ok(AssistantTraceStatus::Stopped),
        _ => Err(super::failure(
            StorageErrorKind::InvalidData,
            "unknown assistant trace status",
        )),
    }
}
fn decode_optional_path(value: Option<Vec<u8>>) -> Result<Option<PathBuf>> {
    value.map(super::records::decode_path).transpose()
}
fn highlights(value: &str, query: &str) -> Vec<std::ops::Range<usize>> {
    if query.is_empty() {
        return Vec::new();
    }
    value
        .to_lowercase()
        .match_indices(query)
        .map(|(start, text)| start..start + text.len())
        .collect()
}
