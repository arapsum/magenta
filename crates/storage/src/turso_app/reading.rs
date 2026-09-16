use super::trace::read_trace;
use super::{
    Attachment, Connection, Conversation, ConversationId, Message, MessageId, MessagePage,
    MessageSequence, PAGE_SIZE, Result, Row, StorageErrorKind, StoredMessage, Timestamp, as_i64,
    as_u64, db, decode_optional_path, params, parse_mode, parse_role, parse_status, scalar_i64,
};

pub(super) async fn ensure_idle(connection: &Connection, id: ConversationId) -> Result<()> {
    let active = scalar_i64(
        connection,
        "SELECT COUNT(*) FROM messages WHERE conversation_id=?1 AND status='streaming'",
        [as_i64(id.0)?],
    )
    .await?;

    if active != 0 {
        Err(super::failure(
            StorageErrorKind::Conflict,
            "conversation already has a streaming message",
        ))
    } else {
        Ok(())
    }
}

pub(super) async fn read_conversation(
    connection: &Connection,
    id: ConversationId,
) -> Result<Conversation> {
    let mut statement = connection
        .prepare("SELECT title,generation,mode,workspace_root FROM conversations WHERE id=?1")
        .await
        .map_err(db)?;

    let mut rows = statement.query([as_i64(id.0)?]).await.map_err(db)?;

    let Some(row) = rows.next().await.map_err(db)? else {
        return Err(super::failure(
            StorageErrorKind::NotFound,
            "conversation does not exist",
        ));
    };
    let generation: String = row.get(1).map_err(db)?;

    Ok(Conversation {
        id,
        title: row.get(0).map_err(db)?,
        generation: serde_json::from_str(&generation).map_err(super::invalid)?,
        mode: parse_mode(&row.get::<String>(2).map_err(db)?)?,
        workspace_root: decode_optional_path(row.get(3).map_err(db)?)?,
    })
}

struct RawMessage {
    id: i64,
    sequence: i64,
    role: String,
    content: String,
    status: String,
    generation: String,
    outcome: Option<String>,
    failure: Option<String>,
    thinking_duration_ms: Option<i64>,
    created_at: i64,
    omitted: i64,
}

fn raw_message(row: &Row) -> Result<RawMessage> {
    Ok(RawMessage {
        id: row.get(0).map_err(db)?,
        sequence: row.get(1).map_err(db)?,
        role: row.get(2).map_err(db)?,
        content: row.get(3).map_err(db)?,
        status: row.get(4).map_err(db)?,
        generation: row.get(5).map_err(db)?,
        outcome: row.get(6).map_err(db)?,
        failure: row.get(7).map_err(db)?,
        thinking_duration_ms: row.get(8).map_err(db)?,
        created_at: row.get(9).map_err(db)?,
        omitted: row.get(10).map_err(db)?,
    })
}

pub(super) async fn read_page(
    connection: &Connection,
    id: ConversationId,
    before: Option<MessageSequence>,
    after: Option<MessageSequence>,
) -> Result<MessagePage> {
    let mut raw = Vec::new();

    if let Some(after) = after {
        let mut statement = connection
            .prepare(
                "SELECT id,sequence,role,content,status,generation,outcome,failure, \
                 thinking_duration_ms,created_at,omitted_context_messages \
                 FROM messages \
                 WHERE conversation_id=?1 AND sequence>?2 \
                 ORDER BY sequence LIMIT 51",
            )
            .await
            .map_err(db)?;

        let mut rows = statement
            .query(params![as_i64(id.0)?, after.0])
            .await
            .map_err(db)?;

        while let Some(row) = rows.next().await.map_err(db)? {
            raw.push(raw_message(&row)?);
        }
    } else {
        let cursor = before.map_or(i64::MAX, |value| value.0);

        let mut statement = connection
            .prepare(
                "SELECT id,sequence,role,content,status,generation,outcome,failure, \
                 thinking_duration_ms,created_at,omitted_context_messages \
                 FROM messages \
                 WHERE conversation_id=?1 AND sequence<?2 \
                 ORDER BY sequence DESC LIMIT 51",
            )
            .await
            .map_err(db)?;

        let mut rows = statement
            .query(params![as_i64(id.0)?, cursor])
            .await
            .map_err(db)?;

        while let Some(row) = rows.next().await.map_err(db)? {
            raw.push(raw_message(&row)?);
        }
        raw.reverse();
    }

    let overflow = raw.len() > PAGE_SIZE;

    if after.is_some() {
        raw.truncate(PAGE_SIZE);
    } else if overflow {
        raw.remove(0);
    }

    build_page(
        connection,
        id,
        raw,
        if after.is_some() {
            None
        } else {
            Some(overflow)
        },
        if after.is_some() {
            Some(overflow)
        } else {
            None
        },
    )
    .await
}

pub(super) async fn read_range(
    connection: &Connection,
    id: ConversationId,
    first: i64,
    last: i64,
) -> Result<MessagePage> {
    let mut statement = connection
        .prepare(
            "SELECT id,sequence,role,content,status,generation,outcome,failure, \
             thinking_duration_ms,created_at,omitted_context_messages \
             FROM messages \
             WHERE conversation_id=?1 AND sequence BETWEEN ?2 AND ?3 \
             ORDER BY sequence",
        )
        .await
        .map_err(db)?;

    let mut rows = statement
        .query(params![as_i64(id.0)?, first, last])
        .await
        .map_err(db)?;

    let mut raw = Vec::new();

    while let Some(row) = rows.next().await.map_err(db)? {
        raw.push(raw_message(&row)?);
    }

    drop(rows);
    drop(statement);

    build_page(connection, id, raw, None, None).await
}

async fn build_page(
    connection: &Connection,
    id: ConversationId,
    raw: Vec<RawMessage>,
    known_older: Option<bool>,
    known_newer: Option<bool>,
) -> Result<MessagePage> {
    let mut messages = Vec::with_capacity(raw.len());

    for row in raw {
        messages.push(hydrate_message(connection, id, row).await?);
    }

    let older_cursor = messages.first().map(|message| message.sequence);
    let newer_cursor = messages.last().map(|message| message.sequence);

    let has_older = match (known_older, older_cursor) {
        (Some(value), _) => value,
        (None, Some(cursor)) => {
            scalar_i64(
                connection,
                "SELECT COUNT(*) FROM messages WHERE conversation_id=?1 AND sequence<?2",
                params![as_i64(id.0)?, cursor.0],
            )
            .await?
                != 0
        }
        _ => false,
    };

    let has_newer = match (known_newer, newer_cursor) {
        (Some(value), _) => value,
        (None, Some(cursor)) => {
            scalar_i64(
                connection,
                "SELECT COUNT(*) FROM messages WHERE conversation_id=?1 AND sequence>?2",
                params![as_i64(id.0)?, cursor.0],
            )
            .await?
                != 0
        }
        _ => false,
    };

    Ok(MessagePage {
        messages,
        older_cursor,
        has_older,
        newer_cursor,
        has_newer,
    })
}

pub(super) async fn read_context(
    connection: &Connection,
    id: ConversationId,
    before: i64,
) -> Result<Vec<Message>> {
    let mut statement = connection
        .prepare(
            "SELECT id,sequence,role,content,status,generation,outcome,failure, \
             thinking_duration_ms,created_at,omitted_context_messages \
             FROM messages \
             WHERE conversation_id=?1 AND sequence<?2 AND status='complete' \
             ORDER BY sequence",
        )
        .await
        .map_err(db)?;

    let mut rows = statement
        .query(params![as_i64(id.0)?, before])
        .await
        .map_err(db)?;

    let mut raw = Vec::new();

    while let Some(row) = rows.next().await.map_err(db)? {
        raw.push(raw_message(&row)?);
    }

    drop(rows);
    drop(statement);

    let mut messages = Vec::with_capacity(raw.len());

    for row in raw {
        messages.push(hydrate_message(connection, id, row).await?.message);
    }

    Ok(messages)
}

async fn hydrate_message(
    connection: &Connection,
    conversation_id: ConversationId,
    raw: RawMessage,
) -> Result<StoredMessage> {
    let id = MessageId(as_u64(raw.id)?);
    let attachments = read_attachments(connection, id).await?;
    let assistant_trace = read_trace(connection, id, raw.thinking_duration_ms).await?;

    let outcome = raw
        .outcome
        .as_deref()
        .map(serde_json::from_str)
        .transpose()
        .map_err(super::invalid)?;

    let failure = raw
        .failure
        .as_deref()
        .map(serde_json::from_str)
        .transpose()
        .map_err(super::invalid)?;

    let generation = serde_json::from_str(&raw.generation).map_err(super::invalid)?;

    let message = Message {
        id,
        conversation_id,
        role: parse_role(&raw.role)?,
        content: raw.content,
        status: parse_status(&raw.status)?,
        attachments,
        generation_outcome: outcome,
        failure,
        assistant_trace,
    };

    Ok(StoredMessage {
        message,
        sequence: MessageSequence(raw.sequence),
        created_at: Timestamp(raw.created_at),
        generation,
        omitted_context_messages: usize::try_from(raw.omitted).map_err(super::invalid)?,
    })
}

async fn read_attachments(connection: &Connection, id: MessageId) -> Result<Vec<Attachment>> {
    let mut statement = connection
        .prepare(
            "SELECT name,source_path,mime_type,byte_size,managed \
             FROM attachments WHERE message_id=?1 ORDER BY position",
        )
        .await
        .map_err(db)?;
    let mut rows = statement.query([as_i64(id.0)?]).await.map_err(db)?;

    let mut result = Vec::new();

    while let Some(row) = rows.next().await.map_err(db)? {
        result.push(Attachment {
            name: row.get(0).map_err(db)?,
            path: super::records::decode_path(row.get(1).map_err(db)?)?,
            mime_type: row.get(2).map_err(db)?,
            byte_size: u64::try_from(row.get::<i64>(3).map_err(db)?).map_err(super::invalid)?,
            managed: row.get::<i64>(4).map_err(db)? != 0,
        });
    }

    Ok(result)
}

pub(super) async fn read_managed_attachments(
    connection: &Connection,
    id: ConversationId,
) -> Result<Vec<Attachment>> {
    let mut statement = connection
        .prepare(
            "SELECT a.name,a.source_path,a.mime_type,a.byte_size,a.managed \
             FROM attachments a \
             JOIN messages m ON m.id=a.message_id \
             WHERE m.conversation_id=?1 AND a.managed=1 \
             ORDER BY m.sequence,a.position",
        )
        .await
        .map_err(db)?;
    let mut rows = statement.query([as_i64(id.0)?]).await.map_err(db)?;

    let mut result = Vec::new();

    while let Some(row) = rows.next().await.map_err(db)? {
        result.push(Attachment {
            name: row.get(0).map_err(db)?,
            path: super::records::decode_path(row.get(1).map_err(db)?)?,
            mime_type: row.get(2).map_err(db)?,
            byte_size: u64::try_from(row.get::<i64>(3).map_err(db)?).map_err(super::invalid)?,
            managed: row.get::<i64>(4).map_err(db)? != 0,
        });
    }

    Ok(result)
}
