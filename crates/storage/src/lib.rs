//! SQLite adapter. Connections and migrations are confined to blocking workers.

mod records;
mod turns;

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
    initialized: Arc<AtomicBool>,
}

impl SqliteConversationStore {
    #[must_use]
    pub fn new(path: PathBuf) -> Self {
        Self {
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
                1 => {}
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
            initialized.store(true, Ordering::Release);
            Ok(())
        }))
    }

    fn summaries(&self) -> StorageFuture<Vec<ConversationSummary>> {
        self.run(|connection| {
            let mut statement = connection
                .prepare(
                    r"
                        SELECT id, title, pinned, created_at, updated_at
                        FROM conversations
                        ORDER BY updated_at DESC, id DESC
                    ",
                )
                .map_err(database_error)?;
            let rows = statement
                .query_map([], |row| {
                    Ok(ConversationSummary {
                        id: ConversationId(row.get(0)?),
                        title: row.get(1)?,
                        pinned: row.get(2)?,
                        created_at: Timestamp(row.get(3)?),
                        updated_at: Timestamp(row.get(4)?),
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
        self.run(move |connection| turns::begin(connection, input))
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
