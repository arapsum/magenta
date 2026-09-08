//! SQLite adapter. Connections and migrations are confined to blocking workers.

mod attachments;
mod managed_attachments;
mod migrations;
mod records;
mod search;
mod settings;
mod turns;

pub use settings::TomlSettingsStore;

use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use magenta_core::{
    AgentActivityKind, AgentActivityRecord, BeginTurn, ConversationId, ConversationPage,
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
            let changed = transaction
                .execute(
                    r"
                        UPDATE messages
                        SET content = ?1, status = ?2, outcome = ?3
                        WHERE id = ?4
                          AND conversation_id = ?5
                          AND status = 'streaming'
                    ",
                    params![
                        message.content,
                        records::status(message.status),
                        outcome,
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
            let managed_attachments = managed_attachments::for_conversation(&transaction, id)?;
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

    fn append_agent_activity(&self, activity: AgentActivityRecord) -> StorageFuture<()> {
        self.run(move |connection| {
            let sequence: i64 = connection
                .query_row(
                    "SELECT COALESCE(MAX(sequence) + 1, 0) FROM agent_activities WHERE run_id = ?1",
                    [activity.run_id.0],
                    |row| row.get(0),
                )
                .map_err(database_error)?;
            connection
                .execute(
                    r"
                        INSERT INTO agent_activities(
                            run_id, sequence, kind, call_id, tool_name,
                            status, summary, detail, created_at
                        )
                        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                    ",
                    params![
                        activity.run_id.0,
                        sequence,
                        activity_kind(&activity.activity.kind),
                        activity.activity.call_id,
                        activity.activity.tool_name,
                        activity.activity.status,
                        activity.activity.summary,
                        activity.activity.detail,
                        now()?,
                    ],
                )
                .map_err(database_error)?;
            Ok(())
        })
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

fn decode_mode(mode: &str) -> rusqlite::Result<magenta_core::ConversationMode> {
    match mode {
        "chat" => Ok(magenta_core::ConversationMode::Chat),
        "agent" => Ok(magenta_core::ConversationMode::Agent),
        other => Err(rusqlite::Error::FromSqlConversionFailure(
            4,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("unknown conversation mode {other}"),
            )),
        )),
    }
}

fn attachment_directory(database_path: &std::path::Path) -> PathBuf {
    database_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join("attachments")
}

const fn activity_kind(kind: &AgentActivityKind) -> &'static str {
    match kind {
        AgentActivityKind::ToolCall => "tool-call",
        AgentActivityKind::ApprovalRequested => "approval-requested",
        AgentActivityKind::ToolResult => "tool-result",
    }
}

const fn agent_run_status(status: magenta_core::MessageStatus) -> &'static str {
    match status {
        magenta_core::MessageStatus::Complete => "completed",
        magenta_core::MessageStatus::Streaming => "running",
        magenta_core::MessageStatus::Stopped => "stopped",
        magenta_core::MessageStatus::Failed => "failed",
    }
}

fn connect(path: &std::path::Path) -> Result<Connection> {
    let connection = Connection::open(path).map_err(database_error)?;
    connection
        .busy_timeout(Duration::from_secs(5))
        .map_err(database_error)?;
    connection
        .pragma_update(None, "foreign_keys", true)
        .map_err(database_error)?;
    connection
        .pragma_update(None, "synchronous", "NORMAL")
        .map_err(database_error)?;
    Ok(connection)
}

fn now() -> Result<i64> {
    i64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(invalid)?
            .as_millis(),
    )
    .map_err(invalid)
}

fn unavailable(source: impl std::error::Error + Send + Sync + 'static) -> StorageError {
    StorageError::new(StorageErrorKind::Unavailable, source)
}

fn invalid(source: impl std::error::Error + Send + Sync + 'static) -> StorageError {
    StorageError::new(StorageErrorKind::InvalidData, source)
}

fn failure(kind: StorageErrorKind, message: &'static str) -> StorageError {
    StorageError::new(kind, std::io::Error::other(message))
}

fn database_error(source: rusqlite::Error) -> StorageError {
    let kind = match &source {
        rusqlite::Error::QueryReturnedNoRows => StorageErrorKind::NotFound,
        rusqlite::Error::FromSqlConversionFailure(..)
        | rusqlite::Error::IntegralValueOutOfRange(..)
        | rusqlite::Error::InvalidColumnType(..) => StorageErrorKind::InvalidData,
        rusqlite::Error::SqliteFailure(error, _) => match error.code {
            rusqlite::ErrorCode::DatabaseCorrupt | rusqlite::ErrorCode::NotADatabase => {
                StorageErrorKind::InvalidData
            }
            rusqlite::ErrorCode::ConstraintViolation => StorageErrorKind::Conflict,
            _ => StorageErrorKind::Unavailable,
        },
        _ => StorageErrorKind::Unavailable,
    };
    StorageError::new(kind, source)
}
