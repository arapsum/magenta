use magenta_core::{
    Attachment, Conversation, ConversationId, Message, MessageId, MessagePage, MessageRole,
    MessageSequence, MessageStatus, StoredMessage, Timestamp,
};
use rusqlite::{Connection, params};

use crate::{Result, database_error, failure, invalid};

pub fn conversation(connection: &Connection, id: ConversationId) -> Result<Conversation> {
    let (title, generation): (String, String) = connection
        .query_row(
            "SELECT title, generation FROM conversations WHERE id = ?1",
            [id.0],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(database_error)?;
    Ok(Conversation {
        id,
        title,
        generation: serde_json::from_str(&generation).map_err(invalid)?,
    })
}

pub fn page(
    connection: &Connection,
    id: ConversationId,
    before: Option<MessageSequence>,
) -> Result<MessagePage> {
    let mut statement = connection.prepare("SELECT id, sequence, role, content, status, generation, outcome, created_at FROM messages WHERE conversation_id = ?1 AND (?2 IS NULL OR sequence < ?2) ORDER BY sequence DESC LIMIT 51").map_err(database_error)?;
    let mut rows = statement
        .query(params![id.0, before.map(|cursor| cursor.0)])
        .map_err(database_error)?;
    let mut messages = Vec::new();
    while let Some(row) = rows.next().map_err(database_error)? {
        messages.push(read_message(connection, id, row)?);
    }
    let has_older = messages.len() > 50;
    messages.truncate(50);
    messages.reverse();
    let older_cursor = messages.first().map(|message| message.sequence);
    Ok(MessagePage {
        messages,
        older_cursor,
        has_older,
    })
}

pub fn context(connection: &Connection, id: ConversationId, before: i64) -> Result<Vec<Message>> {
    let mut statement = connection.prepare("SELECT id, sequence, role, content, status, generation, outcome, created_at FROM messages WHERE conversation_id = ?1 AND sequence < ?2 AND status = 'complete' ORDER BY sequence").map_err(database_error)?;
    let mut rows = statement
        .query(params![id.0, before])
        .map_err(database_error)?;
    let mut messages = Vec::new();
    while let Some(row) = rows.next().map_err(database_error)? {
        messages.push(read_message(connection, id, row)?.message);
    }
    Ok(messages)
}

fn read_message(
    connection: &Connection,
    id: ConversationId,
    row: &rusqlite::Row<'_>,
) -> Result<StoredMessage> {
    let message_id = MessageId(row.get(0).map_err(database_error)?);
    let role: String = row.get(2).map_err(database_error)?;
    let state: String = row.get(4).map_err(database_error)?;
    let generation: String = row.get(5).map_err(database_error)?;
    let outcome: Option<String> = row.get(6).map_err(database_error)?;
    Ok(StoredMessage {
        message: Message {
            id: message_id,
            conversation_id: id,
            role: match role.as_str() {
                "user" => MessageRole::User,
                "assistant" => MessageRole::Assistant,
                _ => {
                    return Err(failure(
                        magenta_core::StorageErrorKind::InvalidData,
                        "unknown message role",
                    ));
                }
            },
            content: row.get(3).map_err(database_error)?,
            status: match state.as_str() {
                "complete" => MessageStatus::Complete,
                "streaming" => MessageStatus::Streaming,
                "stopped" => MessageStatus::Stopped,
                "failed" => MessageStatus::Failed,
                _ => {
                    return Err(failure(
                        magenta_core::StorageErrorKind::InvalidData,
                        "unknown message status",
                    ));
                }
            },
            attachments: attachments(connection, message_id)?,
            generation_outcome: outcome
                .as_deref()
                .map(serde_json::from_str)
                .transpose()
                .map_err(invalid)?,
        },
        sequence: MessageSequence(row.get(1).map_err(database_error)?),
        created_at: Timestamp(row.get(7).map_err(database_error)?),
        generation: serde_json::from_str(&generation).map_err(invalid)?,
    })
}

fn attachments(connection: &Connection, id: MessageId) -> Result<Vec<Attachment>> {
    let mut statement = connection
        .prepare(
            "SELECT name, source_path FROM attachments WHERE message_id = ?1 ORDER BY position",
        )
        .map_err(database_error)?;
    let rows = statement
        .query_map([id.0], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?))
        })
        .map_err(database_error)?;
    rows.map(|row| {
        let (name, bytes) = row.map_err(database_error)?;
        Ok(Attachment {
            name,
            path: decode_path(bytes)?,
        })
    })
    .collect()
}

#[cfg(unix)]
pub fn encode_path(path: &std::path::Path) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt as _;
    path.as_os_str().as_bytes().to_vec()
}

#[cfg(unix)]
fn decode_path(bytes: Vec<u8>) -> Result<std::path::PathBuf> {
    use std::os::unix::ffi::OsStringExt as _;
    if bytes.contains(&0) {
        return Err(failure(
            magenta_core::StorageErrorKind::InvalidData,
            "invalid attachment path",
        ));
    }
    Ok(std::ffi::OsString::from_vec(bytes).into())
}

#[cfg(windows)]
pub(super) fn encode_path(path: &std::path::Path) -> Vec<u8> {
    use std::os::windows::ffi::OsStrExt as _;
    path.as_os_str()
        .encode_wide()
        .flat_map(u16::to_le_bytes)
        .collect()
}

#[cfg(windows)]
fn decode_path(bytes: Vec<u8>) -> Result<std::path::PathBuf> {
    use std::os::windows::ffi::OsStringExt as _;
    if bytes.len() % 2 != 0 {
        return Err(failure(
            magenta_core::StorageErrorKind::InvalidData,
            "invalid attachment path",
        ));
    }
    let wide = bytes
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect::<Vec<_>>();
    Ok(std::ffi::OsString::from_wide(&wide).into())
}

pub const fn status(status: MessageStatus) -> &'static str {
    match status {
        MessageStatus::Complete => "complete",
        MessageStatus::Streaming => "streaming",
        MessageStatus::Stopped => "stopped",
        MessageStatus::Failed => "failed",
    }
}
