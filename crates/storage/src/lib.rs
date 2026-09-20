//! Local Turso, legacy SQLite, managed-attachment, embedding, and settings adapters.
//!
//! [`SqliteConversationStore`] implements conversation and project persistence;
//! initialize it through [`magenta_core::ConversationStore`] before use.
//! [`TomlSettingsStore`] preserves editable preferences and unknown TOML keys.
//! Connections, migrations, and filesystem work are confined to blocking workers.

mod agent_database;
mod attachments;
mod database;
mod embeddings;
mod migrations;
mod records;
mod search;
mod settings;
mod turns;
mod turso_app;

pub use agent_database::TursoAgentDatabase;
pub use embeddings::LocalEmbeddingProvider;
pub use settings::TomlSettingsStore;
pub use turso_app::TursoAppStore;

pub(crate) use database::{
    agent_run_status, attachment_directory, connect, database_error, decode_mode, failure, invalid,
    now, unavailable,
};

use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use magenta_core::{
    AssistantTrace, BeginTurn, CommandId, ConversationId, ConversationPage,
    ConversationSearchResult, ConversationStore, ConversationSummary, Message, MessageId,
    MessagePage, MessageSequence, PreparedTurn, Project, ProjectStore, StorageError,
    StorageErrorKind, StorageFuture, Timestamp,
};
use rusqlite::{Connection, TransactionBehavior, params};

type Result<T> = std::result::Result<T, StorageError>;

#[derive(Clone)]
pub struct SqliteConversationStore {
    path: Arc<PathBuf>,
    attachments_path: Arc<PathBuf>,
    initialized: Arc<AtomicBool>,
}

impl SqliteConversationStore {
    #[must_use]
    pub fn new(path: PathBuf) -> Self {
        Self {
            attachments_path: Arc::new(attachment_directory(&path)),
            path: Arc::new(path),
            initialized: Arc::new(AtomicBool::new(false)),
        }
    }

    fn run<T: Send + 'static>(
        &self,
        operation: impl FnOnce(&mut Connection) -> Result<T> + Send + 'static,
    ) -> StorageFuture<T> {
        let path = Arc::clone(&self.path);
        let initialized = Arc::clone(&self.initialized);
        Box::pin(smol::unblock(move || {
            if !initialized.load(Ordering::Acquire) {
                return Err(failure(
                    StorageErrorKind::Unavailable,
                    "storage has not been initialized",
                ));
            }
            let mut connection = connect(&path)?;
            operation(&mut connection)
        }))
    }
}

impl ConversationStore for SqliteConversationStore {
    fn initialize(&self) -> StorageFuture<()> {
        let path = Arc::clone(&self.path);
        let initialized = Arc::clone(&self.initialized);
        Box::pin(smol::unblock(move || {
            if initialized.load(Ordering::Acquire) {
                return Ok(());
            }
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(unavailable)?;
            }
            let mut connection = connect(&path)?;
            connection
                .pragma_update(None, "journal_mode", "WAL")
                .map_err(database_error)?;
            let transaction = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(database_error)?;
            let version: i64 = transaction
                .pragma_query_value(None, "user_version", |row| row.get(0))
                .map_err(database_error)?;
            if version == 0 {
                transaction
                    .execute_batch(include_str!("schema.sql"))
                    .map_err(database_error)?;
            } else {
                migrations::apply(version, &transaction)?;
            }
            transaction
                .execute(
                    "UPDATE messages SET status = 'stopped' WHERE status = 'streaming'",
                    [],
                )
                .map_err(database_error)?;
            transaction
                .execute(
                    "UPDATE agent_runs SET status = 'stopped', finished_at = ?1 WHERE status = 'running' AND assistant_message_id IN (SELECT id FROM messages WHERE status = 'stopped')",
                    [now()?],
                )
                .map_err(database_error)?;
            transaction.commit().map_err(database_error)?;
            attachments::reconcile(&attachment_directory(&path), &connection)?;
            initialized.store(true, Ordering::Release);
            Ok(())
        }))
    }

    fn summaries(&self) -> StorageFuture<Vec<ConversationSummary>> {
        self.run(|connection| {
            let mut statement = connection
                .prepare(
                    r"
                        SELECT
                            id,
                            title,
                            COALESCE(
                                (
                                    SELECT substr(trim(message.content), 1, 240)
                                    FROM messages AS message
                                    WHERE message.conversation_id = conversation.id
                                      AND message.role = 'user'
                                      AND trim(message.content) <> ''
                                    ORDER BY message.sequence DESC
                                    LIMIT 1
                                ),
                                ''
                            ) AS preview,
                            pinned,
                            mode,
                            workspace_root,
                            created_at,
                            updated_at,
                            json_extract(generation, '$.provider') AS provider
                        FROM conversations AS conversation
                        ORDER BY conversation.updated_at DESC, conversation.id DESC
                    ",
                )
                .map_err(database_error)?;
            let rows = statement
                .query_map([], |row| {
                    Ok(ConversationSummary {
                        id: ConversationId(row.get(0)?),
                        title: row.get(1)?,
                        preview: row.get(2)?,
                        pinned: row.get(3)?,
                        mode: decode_mode(&row.get::<_, String>(4)?)?,
                        workspace_root: row
                            .get::<_, Option<Vec<u8>>>(5)?
                            .map(records::decode_path)
                            .transpose()
                            .map_err(|error| {
                                rusqlite::Error::FromSqlConversionFailure(
                                    5,
                                    rusqlite::types::Type::Blob,
                                    Box::new(error),
                                )
                            })?,
                        created_at: Timestamp(row.get(6)?),
                        updated_at: Timestamp(row.get(7)?),
                        provider: magenta_core::ProviderId(row.get(8)?),
                    })
                })
                .map_err(database_error)?;
            rows.collect::<std::result::Result<Vec<_>, _>>()
                .map_err(database_error)
        })
    }

    fn search(&self, query: String, limit: usize) -> StorageFuture<Vec<ConversationSearchResult>> {
        self.run(move |connection| search::search(connection, &query, limit))
    }

    fn load(&self, id: ConversationId) -> StorageFuture<ConversationPage> {
        self.run(move |connection| {
            let transaction = connection.transaction().map_err(database_error)?;
            let conversation = records::conversation(&transaction, id)?;
            let page = records::page(&transaction, id, None)?;
            transaction.commit().map_err(database_error)?;
            Ok(ConversationPage { conversation, page })
        })
    }

    fn load_around(
        &self,
        id: ConversationId,
        sequence: MessageSequence,
    ) -> StorageFuture<ConversationPage> {
        self.run(move |connection| {
            let transaction = connection.transaction().map_err(database_error)?;
            let conversation = records::conversation(&transaction, id)?;
            let page = records::page_around(&transaction, id, sequence)?;
            transaction.commit().map_err(database_error)?;
            Ok(ConversationPage { conversation, page })
        })
    }

    fn earlier(&self, id: ConversationId, before: MessageSequence) -> StorageFuture<MessagePage> {
        self.run(move |connection| records::page(connection, id, Some(before)))
    }

    fn later(&self, id: ConversationId, after: MessageSequence) -> StorageFuture<MessagePage> {
        self.run(move |connection| records::page_after(connection, id, after))
    }

    fn begin_turn(&self, input: BeginTurn) -> StorageFuture<PreparedTurn> {
        let path = Arc::clone(&self.path);
        let attachments_path = Arc::clone(&self.attachments_path);
        let initialized = Arc::clone(&self.initialized);
        Box::pin(smol::unblock(move || {
            if !initialized.load(Ordering::Acquire) {
                return Err(failure(
                    StorageErrorKind::Unavailable,
                    "storage has not been initialized",
                ));
            }

            let mut connection = connect(&path)?;
            let attachments = attachments::import(&attachments_path, &input.attachments)?;
            match turns::begin(&mut connection, input, attachments.clone()) {
                Ok(prepared) => Ok(prepared),
                Err(error) => {
                    attachments::remove_managed(&attachments_path, &attachments);
                    Err(error)
                }
            }
        }))
    }

    fn begin_regeneration(
        &self,
        id: ConversationId,
        target: MessageId,
        request_overhead_tokens: u64,
    ) -> StorageFuture<PreparedTurn> {
        self.run(move |connection| {
            turns::regenerate(connection, id, target, request_overhead_tokens)
        })
    }

    fn begin_retry(
        &self,
        id: ConversationId,
        target: MessageId,
        generation: magenta_core::GenerationConfig,
        request_overhead_tokens: u64,
    ) -> StorageFuture<PreparedTurn> {
        self.run(move |connection| {
            turns::retry(connection, id, target, generation, request_overhead_tokens)
        })
    }

    fn command_for_response(
        &self,
        conversation_id: ConversationId,
        assistant_message_id: MessageId,
    ) -> StorageFuture<Option<CommandId>> {
        self.run(move |connection| {
            turns::command_for_response(connection, conversation_id, assistant_message_id)
        })
    }

    fn finalize(&self, message: Message) -> StorageFuture<()> {
        self.run(move |connection| {
            if message.role != magenta_core::MessageRole::Assistant
                || message.status == magenta_core::MessageStatus::Streaming
            {
                return Err(failure(
                    StorageErrorKind::InvalidData,
                    "only terminal assistant messages can be finalized",
                ));
            }

            let transaction = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(database_error)?;
            let outcome = message
                .generation_outcome
                .as_ref()
                .map(serde_json::to_string)
                .transpose()
                .map_err(invalid)?;
            let failure_json = message
                .failure
                .as_ref()
                .map(serde_json::to_string)
                .transpose()
                .map_err(invalid)?;
            let changed = transaction
                .execute(
                    r"
                        UPDATE messages
                        SET content = ?1, status = ?2, outcome = ?3, failure = ?4,
                            thinking_duration_ms = ?5
                        WHERE id = ?6
                          AND conversation_id = ?7
                          AND status = 'streaming'
                    ",
                    params![
                        message.content,
                        records::status(message.status),
                        outcome,
                        failure_json,
                        message.assistant_trace.thinking_duration_ms
                            .map(|value| i64::try_from(value).map_err(invalid))
                            .transpose()?,
                        message.id.0,
                        message.conversation_id.0
                    ],
                )
                .map_err(database_error)?;
            if changed != 1 {
                return Err(failure(
                    StorageErrorKind::Conflict,
                    "message is no longer streaming",
                ));
            }
            replace_trace(&transaction, message.id, &message.assistant_trace)?;
            transaction
                .execute(
                    "UPDATE agent_runs SET status = ?1, finished_at = ?2 WHERE assistant_message_id = ?3 AND status = 'running'",
                    params![agent_run_status(message.status), now()?, message.id.0],
                )
                .map_err(database_error)?;
            transaction
                .execute(
                    "UPDATE conversations SET updated_at = ?1 WHERE id = ?2",
                    params![now()?, message.conversation_id.0],
                )
                .map_err(database_error)?;
            transaction.commit().map_err(database_error)
        })
    }

    fn delete(&self, id: ConversationId) -> StorageFuture<()> {
        let path = Arc::clone(&self.path);
        let attachments_path = Arc::clone(&self.attachments_path);
        let initialized = Arc::clone(&self.initialized);
        Box::pin(smol::unblock(move || {
            if !initialized.load(Ordering::Acquire) {
                return Err(failure(
                    StorageErrorKind::Unavailable,
                    "storage has not been initialized",
                ));
            }

            let mut connection = connect(&path)?;
            let transaction = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(database_error)?;
            let managed_attachments = attachments::managed::for_conversation(&transaction, id)?;
            let changed = transaction
                .execute("DELETE FROM conversations WHERE id = ?1", [id.0])
                .map_err(database_error)?;
            if changed == 0 {
                return Err(failure(
                    StorageErrorKind::NotFound,
                    "conversation does not exist",
                ));
            }
            transaction.commit().map_err(database_error)?;
            attachments::remove_managed(&attachments_path, &managed_attachments);
            Ok(())
        }))
    }

    fn rename(&self, id: ConversationId, title: String) -> StorageFuture<()> {
        self.run(move |connection| {
            let changed = connection
                .execute(
                    "UPDATE conversations SET title = ?1 WHERE id = ?2",
                    params![title, id.0],
                )
                .map_err(database_error)?;
            if changed == 0 {
                return Err(failure(
                    StorageErrorKind::NotFound,
                    "conversation does not exist",
                ));
            }
            Ok(())
        })
    }

    fn rename_if_current(
        &self,
        id: ConversationId,
        current: String,
        title: String,
    ) -> StorageFuture<bool> {
        self.run(move |connection| {
            let changed = connection
                .execute(
                    "UPDATE conversations SET title = ?1 WHERE id = ?2 AND title = ?3",
                    params![title, id.0, current],
                )
                .map_err(database_error)?;
            Ok(changed == 1)
        })
    }

    fn set_pinned(&self, id: ConversationId, pinned: bool) -> StorageFuture<()> {
        self.run(move |connection| {
            let changed = connection
                .execute(
                    "UPDATE conversations SET pinned = ?1 WHERE id = ?2",
                    params![pinned, id.0],
                )
                .map_err(database_error)?;
            if changed == 0 {
                return Err(failure(
                    StorageErrorKind::NotFound,
                    "conversation does not exist",
                ));
            }
            Ok(())
        })
    }

    fn upsert_assistant_trace(
        &self,
        message_id: MessageId,
        trace: AssistantTrace,
    ) -> StorageFuture<()> {
        self.run(move |connection| {
            let transaction = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(database_error)?;
            let duration = trace
                .thinking_duration_ms
                .map(|value| i64::try_from(value).map_err(invalid))
                .transpose()?;
            let changed = transaction
                .execute(
                    "UPDATE messages SET thinking_duration_ms = ?1 WHERE id = ?2",
                    params![duration, message_id.0],
                )
                .map_err(database_error)?;
            if changed == 0 {
                return Err(failure(
                    StorageErrorKind::NotFound,
                    "assistant message does not exist",
                ));
            }
            replace_trace(&transaction, message_id, &trace)?;
            transaction.commit().map_err(database_error)
        })
    }
}

fn replace_trace(
    transaction: &rusqlite::Transaction<'_>,
    message_id: MessageId,
    trace: &AssistantTrace,
) -> Result<()> {
    transaction
        .execute(
            "DELETE FROM assistant_traces WHERE assistant_message_id = ?1",
            [message_id.0],
        )
        .map_err(database_error)?;
    for entry in &trace.entries {
        transaction
            .execute(
                "INSERT INTO assistant_traces( \
                 assistant_message_id, trace_key, sequence, kind, status, title, tool_name, \
                 input, output, started_at, finished_at \
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![
                    message_id.0,
                    entry.key,
                    i64::try_from(entry.sequence).map_err(invalid)?,
                    trace_kind(entry.kind),
                    trace_status(entry.status),
                    entry.title,
                    entry.tool_name,
                    entry.input,
                    entry.output,
                    entry.started_at.map(|value| value.0),
                    entry.finished_at.map(|value| value.0),
                ],
            )
            .map_err(database_error)?;
    }
    Ok(())
}

const fn trace_kind(kind: magenta_core::AssistantTraceKind) -> &'static str {
    match kind {
        magenta_core::AssistantTraceKind::ReasoningSummary => "reasoning_summary",
        magenta_core::AssistantTraceKind::Tool => "tool",
    }
}

const fn trace_status(status: magenta_core::AssistantTraceStatus) -> &'static str {
    match status {
        magenta_core::AssistantTraceStatus::Streaming => "streaming",
        magenta_core::AssistantTraceStatus::Requested => "requested",
        magenta_core::AssistantTraceStatus::Running => "running",
        magenta_core::AssistantTraceStatus::AwaitingApproval => "awaiting_approval",
        magenta_core::AssistantTraceStatus::Completed => "completed",
        magenta_core::AssistantTraceStatus::Rejected => "rejected",
        magenta_core::AssistantTraceStatus::Failed => "failed",
        magenta_core::AssistantTraceStatus::Stopped => "stopped",
    }
}

impl ProjectStore for SqliteConversationStore {
    fn projects(&self) -> StorageFuture<Vec<Project>> {
        self.run(|connection| {
            let mut statement = connection
                .prepare(
                    "SELECT name, root, added_at, last_opened_at FROM projects \
                     ORDER BY last_opened_at DESC, name COLLATE NOCASE",
                )
                .map_err(database_error)?;
            let rows = statement
                .query_map([], |row| {
                    let root = records::decode_path(row.get(1)?).map_err(|error| {
                        rusqlite::Error::FromSqlConversionFailure(
                            1,
                            rusqlite::types::Type::Blob,
                            Box::new(error),
                        )
                    })?;
                    Ok(Project {
                        name: row.get(0)?,
                        root,
                        added_at: Timestamp(row.get(2)?),
                        last_opened_at: Timestamp(row.get(3)?),
                    })
                })
                .map_err(database_error)?;
            rows.collect::<std::result::Result<Vec<_>, _>>()
                .map_err(database_error)
        })
    }

    fn upsert_project(&self, project: Project) -> StorageFuture<()> {
        self.run(move |connection| {
            connection
                .execute(
                    r"
                        INSERT INTO projects(root, name, added_at, last_opened_at)
                        VALUES (?1, ?2, ?3, ?4)
                        ON CONFLICT(root) DO UPDATE SET
                            name = excluded.name,
                            last_opened_at = excluded.last_opened_at
                    ",
                    params![
                        records::encode_path(&project.root),
                        project.name,
                        project.added_at.0,
                        project.last_opened_at.0,
                    ],
                )
                .map_err(database_error)?;
            Ok(())
        })
    }

    fn remove_project(&self, root: PathBuf) -> StorageFuture<()> {
        self.run(move |connection| {
            connection
                .execute(
                    "DELETE FROM projects WHERE root = ?1",
                    [records::encode_path(&root)],
                )
                .map_err(database_error)?;
            Ok(())
        })
    }
}
