use std::{
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use magenta_core::{StorageError, StorageErrorKind};
use rusqlite::Connection;

use super::Result;

pub fn decode_mode(mode: &str) -> rusqlite::Result<magenta_core::ConversationMode> {
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

pub fn attachment_directory(database_path: &Path) -> PathBuf {
    database_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("attachments")
}

pub const fn agent_run_status(status: magenta_core::MessageStatus) -> &'static str {
    match status {
        magenta_core::MessageStatus::Complete => "completed",
        magenta_core::MessageStatus::Streaming => "running",
        magenta_core::MessageStatus::Stopped => "stopped",
        magenta_core::MessageStatus::Failed => "failed",
    }
}

pub fn connect(path: &Path) -> Result<Connection> {
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

pub fn now() -> Result<i64> {
    i64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(invalid)?
            .as_millis(),
    )
    .map_err(invalid)
}

pub fn unavailable(source: impl std::error::Error + Send + Sync + 'static) -> StorageError {
    StorageError::new(StorageErrorKind::Unavailable, source)
}

pub fn invalid(source: impl std::error::Error + Send + Sync + 'static) -> StorageError {
    StorageError::new(StorageErrorKind::InvalidData, source)
}

pub fn failure(kind: StorageErrorKind, message: &'static str) -> StorageError {
    StorageError::new(kind, std::io::Error::other(message))
}

pub fn database_error(source: rusqlite::Error) -> StorageError {
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
