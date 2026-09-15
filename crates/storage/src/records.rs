use magenta_core::{
    AssistantTrace, AssistantTraceEntry, AssistantTraceKind, AssistantTraceStatus, Attachment,
    Conversation, ConversationId, ConversationMode, Message, MessageId, MessagePage, MessageRole,
    MessageSequence, MessageStatus, StoredMessage, Timestamp,
};
use rusqlite::{Connection, params};

use crate::{Result, database_error, failure, invalid};

pub fn conversation(connection: &Connection, id: ConversationId) -> Result<Conversation> {
    let (title, generation, mode, workspace_root): (String, String, String, Option<Vec<u8>>) =
        connection
            .query_row(
                "SELECT title, generation, mode, workspace_root FROM conversations WHERE id = ?1",
                [id.0],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .map_err(database_error)?;
    let mode = match mode.as_str() {
        "chat" => ConversationMode::Chat,
        "agent" => ConversationMode::Agent,
        _ => {
            return Err(failure(
                magenta_core::StorageErrorKind::InvalidData,
                "unknown conversation mode",
            ));
        }
    };
    Ok(Conversation {
        id,
        title,
        generation: serde_json::from_str(&generation).map_err(invalid)?,
        mode,
        workspace_root: workspace_root.map(decode_path).transpose()?,
    })
}

pub fn page(
    connection: &Connection,
    id: ConversationId,
    before: Option<MessageSequence>,
) -> Result<MessagePage> {
    let mut statement = connection
        .prepare(
            r"
                SELECT id, sequence, role, content, status, generation, outcome, failure,
                       thinking_duration_ms, created_at, omitted_context_messages
                FROM messages
                WHERE conversation_id = ?1
                  AND (?2 IS NULL OR sequence < ?2)
                ORDER BY sequence DESC
                LIMIT 51
            ",
        )
        .map_err(database_error)?;
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
    let newer_cursor = messages.last().map(|message| message.sequence);
    let has_newer = if let Some(cursor) = newer_cursor {
        has_messages_after(connection, id, cursor)?
    } else {
        false
    };
    Ok(MessagePage {
        messages,
        older_cursor,
        has_older,
        newer_cursor,
        has_newer,
    })
}

pub fn page_after(
    connection: &Connection,
    id: ConversationId,
    after: MessageSequence,
) -> Result<MessagePage> {
    let mut statement = connection
        .prepare(
            r"
                SELECT id, sequence, role, content, status, generation, outcome, failure,
                       thinking_duration_ms, created_at, omitted_context_messages
                FROM messages
                WHERE conversation_id = ?1 AND sequence > ?2
                ORDER BY sequence
                LIMIT 51
            ",
        )
        .map_err(database_error)?;
    let mut rows = statement
        .query(params![id.0, after.0])
        .map_err(database_error)?;
    let mut messages = Vec::new();
    while let Some(row) = rows.next().map_err(database_error)? {
        messages.push(read_message(connection, id, row)?);
    }
    let has_newer = messages.len() > 50;
    messages.truncate(50);
    let older_cursor = messages.first().map(|message| message.sequence);
    let newer_cursor = messages.last().map(|message| message.sequence);
    let has_older = older_cursor.is_some_and(|cursor| cursor.0 > 0);
    Ok(MessagePage {
        messages,
        older_cursor,
        has_older,
        newer_cursor,
        has_newer,
    })
}

pub fn page_around(
    connection: &Connection,
    id: ConversationId,
    target: MessageSequence,
) -> Result<MessagePage> {
    let first = target.0.saturating_sub(24).max(0);
    let last = target.0.saturating_add(25);
    let mut statement = connection
        .prepare(
            r"
                SELECT id, sequence, role, content, status, generation, outcome, failure,
                       thinking_duration_ms, created_at, omitted_context_messages
                FROM messages
                WHERE conversation_id = ?1
                  AND sequence BETWEEN ?2 AND ?3
                ORDER BY sequence
            ",
        )
        .map_err(database_error)?;
    let mut rows = statement
        .query(params![id.0, first, last])
        .map_err(database_error)?;
    let mut messages = Vec::new();
    while let Some(row) = rows.next().map_err(database_error)? {
        messages.push(read_message(connection, id, row)?);
    }
    let Some(older_cursor) = messages.first().map(|message| message.sequence) else {
        return Err(failure(
            magenta_core::StorageErrorKind::NotFound,
            "search result message does not exist",
        ));
    };
    let has_older = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM messages WHERE conversation_id = ?1 AND sequence < ?2)",
            params![id.0, older_cursor.0],
            |row| row.get(0),
        )
        .map_err(database_error)?;
    let newer_cursor = messages.last().map(|message| message.sequence);
    let has_newer = match newer_cursor {
        Some(cursor) => has_messages_after(connection, id, cursor)?,
        None => false,
    };
    Ok(MessagePage {
        messages,
        older_cursor: Some(older_cursor),
        has_older,
        newer_cursor,
        has_newer,
    })
}

pub fn context(connection: &Connection, id: ConversationId, before: i64) -> Result<Vec<Message>> {
    let mut statement = connection
        .prepare(
            r"
                SELECT id, sequence, role, content, status, generation, outcome, failure,
                       thinking_duration_ms, created_at, omitted_context_messages
                FROM messages
                WHERE conversation_id = ?1
                  AND sequence < ?2
                  AND status = 'complete'
                ORDER BY sequence
            ",
        )
        .map_err(database_error)?;
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
    let failure_json: Option<String> = row.get(7).map_err(database_error)?;

    let role = match role.as_str() {
        "user" => MessageRole::User,
        "assistant" => MessageRole::Assistant,
        _ => {
            return Err(failure(
                magenta_core::StorageErrorKind::InvalidData,
                "unknown message role",
            ));
        }
    };
    let status = match state.as_str() {
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
    };
    let content = row.get(3).map_err(database_error)?;
    let attachments = attachments(connection, message_id)?;
    let thinking_duration_ms: Option<i64> = row.get(8).map_err(database_error)?;
    let assistant_trace = assistant_trace(connection, message_id, thinking_duration_ms)?;
    let generation_outcome = outcome
        .as_deref()
        .map(serde_json::from_str)
        .transpose()
        .map_err(invalid)?;
    let failure = failure_json
        .as_deref()
        .map(serde_json::from_str)
        .transpose()
        .map_err(invalid)?;
    let sequence = MessageSequence(row.get(1).map_err(database_error)?);
    let created_at = Timestamp(row.get(9).map_err(database_error)?);
    let generation = serde_json::from_str(&generation).map_err(invalid)?;
    let omitted_context_messages =
        usize::try_from(row.get::<_, i64>(10).map_err(database_error)?).map_err(invalid)?;

    Ok(StoredMessage {
        message: Message {
            id: message_id,
            conversation_id: id,
            role,
            content,
            status,
            attachments,
            generation_outcome,
            failure,
            assistant_trace,
        },
        sequence,
        created_at,
        generation,
        omitted_context_messages,
    })
}

fn has_messages_after(
    connection: &Connection,
    id: ConversationId,
    cursor: MessageSequence,
) -> Result<bool> {
    connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM messages WHERE conversation_id = ?1 AND sequence > ?2)",
            params![id.0, cursor.0],
            |row| row.get(0),
        )
        .map_err(database_error)
}

fn assistant_trace(
    connection: &Connection,
    message_id: MessageId,
    thinking_duration_ms: Option<i64>,
) -> Result<AssistantTrace> {
    let mut statement = connection
        .prepare(
            r"
                SELECT trace_key, sequence, kind, status, title, tool_name,
                       input, output, started_at, finished_at
                FROM assistant_traces
                WHERE assistant_message_id = ?1
                ORDER BY sequence
            ",
        )
        .map_err(database_error)?;
    let rows = statement
        .query_map([message_id.0], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, String>(7)?,
                row.get::<_, Option<i64>>(8)?,
                row.get::<_, Option<i64>>(9)?,
            ))
        })
        .map_err(database_error)?;
    let entries = rows
        .map(|row| {
            let (
                key,
                sequence,
                kind,
                status,
                title,
                tool_name,
                input,
                output,
                started_at,
                finished_at,
            ) = row.map_err(database_error)?;
            let kind = match kind.as_str() {
                "reasoning_summary" => AssistantTraceKind::ReasoningSummary,
                "tool" => AssistantTraceKind::Tool,
                _ => {
                    return Err(failure(
                        magenta_core::StorageErrorKind::InvalidData,
                        "unknown assistant trace kind",
                    ));
                }
            };
            let status = match status.as_str() {
                "streaming" => AssistantTraceStatus::Streaming,
                "requested" => AssistantTraceStatus::Requested,
                "running" => AssistantTraceStatus::Running,
                "awaiting_approval" => AssistantTraceStatus::AwaitingApproval,
                "completed" => AssistantTraceStatus::Completed,
                "rejected" => AssistantTraceStatus::Rejected,
                "failed" => AssistantTraceStatus::Failed,
                "stopped" => AssistantTraceStatus::Stopped,
                _ => {
                    return Err(failure(
                        magenta_core::StorageErrorKind::InvalidData,
                        "unknown assistant trace status",
                    ));
                }
            };
            Ok(AssistantTraceEntry {
                key,
                sequence: u64::try_from(sequence).map_err(invalid)?,
                kind,
                status,
                title,
                tool_name,
                input,
                output,
                started_at: started_at.map(Timestamp),
                finished_at: finished_at.map(Timestamp),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(AssistantTrace {
        entries,
        thinking_duration_ms: thinking_duration_ms
            .map(|value| u64::try_from(value).map_err(invalid))
            .transpose()?,
    })
}

fn attachments(connection: &Connection, id: MessageId) -> Result<Vec<Attachment>> {
    let mut statement = connection
        .prepare(
            r"
                SELECT name, source_path, mime_type, byte_size, managed
                FROM attachments
                WHERE message_id = ?1
                ORDER BY position
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
        let (name, bytes, mime_type, byte_size, managed) = row.map_err(database_error)?;
        Ok(Attachment {
            name,
            path: decode_path(bytes)?,
            mime_type,
            byte_size: u64::try_from(byte_size).map_err(invalid)?,
            managed,
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
pub fn decode_path(bytes: Vec<u8>) -> Result<std::path::PathBuf> {
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
pub fn decode_path(bytes: Vec<u8>) -> Result<std::path::PathBuf> {
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
