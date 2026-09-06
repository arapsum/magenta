use magenta_core::{
    Attachment, BeginTurn, Conversation, ConversationId, Message, MessageId, MessageRole,
    MessageStatus, PreparedTurn, StorageErrorKind,
};
use rusqlite::{Connection, Transaction, TransactionBehavior, params};

use crate::{Result, database_error, failure, invalid, now, records};

pub fn begin(connection: &mut Connection, input: BeginTurn) -> Result<PreparedTurn> {
    let BeginTurn {
        conversation_id,
        title,
        prompt,
        attachments,
        generation: generation_config,
    } = input;

    if prompt.trim().is_empty() {
        return Err(failure(StorageErrorKind::InvalidData, "empty prompt"));
    }

    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(database_error)?;

    let timestamp = now()?;
    let generation = serde_json::to_string(&generation_config).map_err(invalid)?;

    let conversation = if let Some(id) = conversation_id {
        let mut conversation = records::conversation(&transaction, id)?;
        ensure_idle(&transaction, id)?;

        transaction
            .execute(
                "UPDATE conversations SET generation = ?1, updated_at = ?2 WHERE id = ?3",
                params![generation, timestamp, id.0],
            )
            .map_err(database_error)?;
        conversation.generation = generation_config;

        conversation
    } else {
        transaction
            .execute(
                r"
                    INSERT INTO conversations(title, generation, created_at, updated_at)
                    VALUES (?1, ?2, ?3, ?3)
                ",
                params![title, generation, timestamp],
            )
            .map_err(database_error)?;

        Conversation {
            id: ConversationId(u64::try_from(transaction.last_insert_rowid()).map_err(invalid)?),
            title,
            generation: generation_config,
        }
    };

    let sequence: i64 = transaction
        .query_row(
            "SELECT COALESCE(MAX(sequence) + 1, 0) FROM messages WHERE conversation_id = ?1",
            [conversation.id.0],
            |row| row.get(0),
        )
        .map_err(database_error)?;

    let mut context = records::context(&transaction, conversation.id, sequence)?;

    let user_message = insert_user_message(
        &transaction,
        conversation.id,
        sequence,
        prompt,
        attachments,
        &generation,
        timestamp,
    )?;

    let assistant_message = insert_assistant_message(
        &transaction,
        conversation.id,
        sequence,
        &generation,
        timestamp,
    )?;

    context.push(user_message.clone());
    transaction.commit().map_err(database_error)?;

    Ok(PreparedTurn {
        conversation,
        user_message,
        assistant_message,
        context,
    })
}

fn insert_user_message(
    transaction: &Transaction<'_>,
    conversation_id: ConversationId,
    sequence: i64,
    prompt: String,
    attachments: Vec<Attachment>,
    generation: &str,
    timestamp: i64,
) -> Result<Message> {
    transaction
        .execute(
            r"
                INSERT INTO messages(
                    conversation_id, sequence, role, content, status, generation, created_at
                )
                VALUES (?1, ?2, 'user', ?3, 'complete', ?4, ?5)
            ",
            params![conversation_id.0, sequence, &prompt, generation, timestamp],
        )
        .map_err(database_error)?;
    let user_id = MessageId(u64::try_from(transaction.last_insert_rowid()).map_err(invalid)?);

    for (index, attachment) in attachments.iter().enumerate() {
        let position = i64::try_from(index).map_err(invalid)?;
        let source_path = records::encode_path(&attachment.path);
        transaction
            .execute(
                r"
                    INSERT INTO attachments(message_id, position, name, source_path)
                    VALUES (?1, ?2, ?3, ?4)
                ",
                params![user_id.0, position, &attachment.name, source_path],
            )
            .map_err(database_error)?;
    }

    Ok(Message {
        id: user_id,
        conversation_id,
        role: MessageRole::User,
        content: prompt,
        status: MessageStatus::Complete,
        attachments,
        generation_outcome: None,
    })
}

fn insert_assistant_message(
    transaction: &Transaction<'_>,
    conversation_id: ConversationId,
    sequence: i64,
    generation: &str,
    timestamp: i64,
) -> Result<Message> {
    let assistant_sequence = sequence
        .checked_add(1)
        .ok_or_else(|| failure(StorageErrorKind::InvalidData, "message sequence overflow"))?;
    transaction
        .execute(
            r"
                INSERT INTO messages(
                    conversation_id, sequence, role, content, status, generation, created_at
                )
                VALUES (?1, ?2, 'assistant', '', 'streaming', ?3, ?4)
            ",
            params![conversation_id.0, assistant_sequence, generation, timestamp],
        )
        .map_err(database_error)?;

    Ok(Message {
        id: MessageId(u64::try_from(transaction.last_insert_rowid()).map_err(invalid)?),
        conversation_id,
        role: MessageRole::Assistant,
        content: String::new(),
        status: MessageStatus::Streaming,
        attachments: Vec::new(),
        generation_outcome: None,
    })
}

pub fn regenerate(
    connection: &mut Connection,
    id: ConversationId,
    target: MessageId,
) -> Result<PreparedTurn> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(database_error)?;

    let conversation = records::conversation(&transaction, id)?;

    ensure_idle(&transaction, id)?;

    let (sequence, role): (i64, String) = transaction
        .query_row(
            "SELECT sequence, role FROM messages WHERE conversation_id = ?1 AND id = ?2",
            params![id.0, target.0],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(database_error)?;

    if role != "assistant" {
        return Err(failure(
            StorageErrorKind::InvalidData,
            "regeneration target is not an assistant",
        ));
    }

    let context = records::context(&transaction, id, sequence)?;
    let user_message = context
        .iter()
        .rev()
        .find(|message| message.role == MessageRole::User)
        .cloned()
        .ok_or_else(|| failure(StorageErrorKind::InvalidData, "missing user context"))?;

    let generation = serde_json::to_string(&conversation.generation).map_err(invalid)?;

    transaction
        .execute(
            r"
                UPDATE messages
                SET content = '', status = 'streaming', outcome = NULL, generation = ?1
                WHERE id = ?2
            ",
            params![generation, target.0],
        )
        .map_err(database_error)?;

    transaction
        .execute(
            "UPDATE conversations SET updated_at = ?1 WHERE id = ?2",
            params![now()?, id.0],
        )
        .map_err(database_error)?;

    let assistant_message = Message {
        id: target,
        conversation_id: id,
        role: MessageRole::Assistant,
        content: String::new(),
        status: MessageStatus::Streaming,
        attachments: Vec::new(),
        generation_outcome: None,
    };

    transaction.commit().map_err(database_error)?;

    Ok(PreparedTurn {
        conversation,
        user_message,
        assistant_message,
        context,
    })
}

fn ensure_idle(connection: &Connection, id: ConversationId) -> Result<()> {
    let streaming: bool = connection
        .query_row(
            r"
                SELECT EXISTS(
                    SELECT 1
                    FROM messages
                    WHERE conversation_id = ?1
                      AND status = 'streaming'
                )
            ",
            [id.0],
            |row| row.get(0),
        )
        .map_err(database_error)?;

    if streaming {
        return Err(failure(
            StorageErrorKind::Conflict,
            "conversation already has a streaming response",
        ));
    }
    Ok(())
}
