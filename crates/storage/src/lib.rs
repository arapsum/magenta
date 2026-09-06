//! SQLite adapter. Connections and migrations are confined to blocking workers.

mod attachments;
mod records;
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
    BeginTurn, ConversationId, ConversationPage, ConversationStore, ConversationSummary, Message,
    MessageId, MessagePage, MessageSequence, PreparedTurn, StorageError, StorageErrorKind,
    StorageFuture, Timestamp,
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
            match version {
                0 => transaction
                    .execute_batch(include_str!("schema.sql"))
                    .map_err(database_error)?,
                1 => migrate_v1_to_v2(&transaction)?,
                2 => {}
                _ => {
                    return Err(failure(
                        StorageErrorKind::UnsupportedVersion,
                        "unsupported database schema version",
                    ));
                }
            }
            transaction
                .execute(
                    "UPDATE messages SET status = 'stopped' WHERE status = 'streaming'",
                    [],
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
                            created_at,
                            updated_at
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
                        created_at: Timestamp(row.get(4)?),
                        updated_at: Timestamp(row.get(5)?),
                    })
                })
                .map_err(database_error)?;
            rows.collect::<std::result::Result<Vec<_>, _>>()
                .map_err(database_error)
        })
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

    fn earlier(&self, id: ConversationId, before: MessageSequence) -> StorageFuture<MessagePage> {
        self.run(move |connection| records::page(connection, id, Some(before)))
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
    ) -> StorageFuture<PreparedTurn> {
        self.run(move |connection| turns::regenerate(connection, id, target))
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
            let managed_attachments = managed_attachments(&transaction, id)?;
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
}

fn attachment_directory(database_path: &std::path::Path) -> PathBuf {
    database_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join("attachments")
}

fn migrate_v1_to_v2(transaction: &rusqlite::Transaction<'_>) -> Result<()> {
    transaction
        .execute_batch(
            r"
                ALTER TABLE attachments
                    ADD COLUMN mime_type TEXT NOT NULL DEFAULT 'application/octet-stream';
                ALTER TABLE attachments
                    ADD COLUMN byte_size INTEGER NOT NULL DEFAULT 0 CHECK (byte_size >= 0);
                ALTER TABLE attachments
                    ADD COLUMN managed INTEGER NOT NULL DEFAULT 0 CHECK (managed IN (0, 1));
                PRAGMA user_version = 2;
            ",
        )
        .map_err(database_error)
}

fn managed_attachments(
    connection: &Connection,
    id: ConversationId,
) -> Result<Vec<magenta_core::Attachment>> {
    let mut statement = connection
        .prepare(
            r"
                SELECT attachment.name, attachment.source_path, attachment.mime_type,
                       attachment.byte_size, attachment.managed
                FROM attachments AS attachment
                INNER JOIN messages AS message ON message.id = attachment.message_id
                WHERE message.conversation_id = ?1
                  AND attachment.managed = 1
                ORDER BY message.sequence, attachment.position
            ",
        )
        .map_err(database_error)?;
    let rows = statement
        .query_map([id.0], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Vec<u8>>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, bool>(4)?,
            ))
        })
        .map_err(database_error)?;
    rows.map(|row| {
        let (name, path, mime_type, byte_size, managed) = row.map_err(database_error)?;
        Ok(magenta_core::Attachment {
            name,
            path: records::decode_path(path)?,
            mime_type,
            byte_size: u64::try_from(byte_size).map_err(invalid)?,
            managed,
        })
    })
    .collect()
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
