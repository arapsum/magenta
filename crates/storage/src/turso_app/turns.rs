use super::reading::{ensure_idle, read_context, read_conversation};
use super::{
    AgentRunId, AssistantTrace, Attachment, BeginTurn, CommandId, Connection, ContextBudgetReport,
    Conversation, ConversationId, ConversationMode, GenerationConfig, Message, MessageId,
    MessageRole, MessageSequence, MessageStatus, PathBuf, PreparedTurn, Result, StorageError,
    StorageErrorKind, Transaction, TransactionBehavior, as_i64, as_u64, db, mode_name, params,
    scalar_i64, select_context,
};

pub(super) async fn begin_turn(
    connection: &mut Connection,
    input: BeginTurn,
    attachments: Vec<Attachment>,
) -> Result<PreparedTurn> {
    let BeginTurn {
        conversation_id,
        title,
        prompt,
        command_id,
        attachments: _,
        generation,
        mode,
        workspace_root,
        request_overhead_tokens,
    } = input;

    if prompt.trim().is_empty() && attachments.is_empty() && command_id.is_none() {
        return Err(super::failure(
            StorageErrorKind::InvalidData,
            "empty prompt",
        ));
    }

    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .await
        .map_err(db)?;

    let timestamp = super::now()?;
    let conversation_setup = begin_turn_conversation(
        &transaction,
        conversation_id,
        title,
        generation,
        mode,
        workspace_root,
        timestamp,
    )
    .await?;
    let conversation = conversation_setup.conversation;
    let generation_json = conversation_setup.generation_json;

    let sequence = scalar_i64(
        &transaction,
        "SELECT COALESCE(MAX(sequence)+1,0) FROM messages WHERE conversation_id=?1",
        [as_i64(conversation.id.0)?],
    )
    .await?;

    let mut context = read_context(&transaction, conversation.id, sequence).await?;

    let user_message = insert_user(
        &transaction,
        UserMessageInput {
            conversation_id: conversation.id,
            sequence,
            content: prompt,
            attachments,
            command_id,
            generation: &generation_json,
            timestamp,
        },
    )
    .await?;

    context.push(user_message.clone());

    let (context, context_report) = select_context(
        &context,
        conversation.generation.limits,
        request_overhead_tokens,
    )
    .map_err(|error| StorageError::new(StorageErrorKind::ContextTooLarge, error))?;

    let assistant_message = insert_assistant(
        &transaction,
        conversation.id,
        sequence.checked_add(1).ok_or_else(|| {
            super::failure(StorageErrorKind::InvalidData, "message sequence overflow")
        })?,
        &generation_json,
        timestamp,
        context_report.omitted_messages,
    )
    .await?;

    let agent_run_id =
        insert_agent_run(&transaction, &conversation, &assistant_message, timestamp).await?;

    transaction.commit().await.map_err(db)?;
    Ok(PreparedTurn {
        conversation,
        user_message,
        assistant_message,
        context,
        agent_run_id,
        user_sequence: MessageSequence(sequence),
        assistant_sequence: MessageSequence(sequence + 1),
        context_report,
    })
}

struct BeginTurnConversation {
    conversation: Conversation,
    generation_json: String,
}

async fn begin_turn_conversation(
    transaction: &Transaction<'_>,
    conversation_id: Option<ConversationId>,
    title: String,
    generation: GenerationConfig,
    mode: ConversationMode,
    workspace_root: Option<PathBuf>,
    timestamp: i64,
) -> Result<BeginTurnConversation> {
    let generation_json = serde_json::to_string(&generation).map_err(super::invalid)?;

    if let Some(id) = conversation_id {
        ensure_idle(transaction, id).await?;

        let mut conversation = read_conversation(transaction, id).await?;

        transaction
            .execute(
                "UPDATE conversations SET generation=?1,mode=?2,workspace_root=?3, \
                 updated_at=?4 WHERE id=?5",
                params![
                    generation_json.as_str(),
                    mode_name(&mode),
                    workspace_root
                        .as_ref()
                        .map(|path| super::records::encode_path(path)),
                    timestamp,
                    as_i64(id.0)?,
                ],
            )
            .await
            .map_err(db)?;

        conversation.generation = generation;
        conversation.mode = mode;
        conversation.workspace_root = workspace_root;

        return Ok(BeginTurnConversation {
            conversation,
            generation_json,
        });
    }

    transaction
        .execute(
            "INSERT INTO conversations( \
             title,generation,mode,workspace_root,created_at,updated_at \
             ) VALUES (?1,?2,?3,?4,?5,?5)",
            params![
                title.as_str(),
                generation_json.as_str(),
                mode_name(&mode),
                workspace_root
                    .as_ref()
                    .map(|path| super::records::encode_path(path)),
                timestamp,
            ],
        )
        .await
        .map_err(db)?;

    Ok(BeginTurnConversation {
        conversation: Conversation {
            id: ConversationId(as_u64(transaction.last_insert_rowid())?),
            title,
            generation,
            mode,
            workspace_root,
        },
        generation_json,
    })
}

struct RegenerationContext {
    sequence: i64,
    user_message: Message,
    context: Vec<Message>,
    context_report: ContextBudgetReport,
}

async fn read_regeneration_context(
    transaction: &Transaction<'_>,
    id: ConversationId,
    target: MessageId,
    conversation: &Conversation,
    overhead: u64,
) -> Result<RegenerationContext> {
    let mut statement = transaction
        .prepare("SELECT sequence,role FROM messages WHERE conversation_id=?1 AND id=?2")
        .await
        .map_err(db)?;

    let mut rows = statement
        .query(params![as_i64(id.0)?, as_i64(target.0)?])
        .await
        .map_err(db)?;

    let Some(row) = rows.next().await.map_err(db)? else {
        return Err(super::failure(
            StorageErrorKind::NotFound,
            "message does not exist",
        ));
    };

    let sequence: i64 = row.get(0).map_err(db)?;

    if row.get::<String>(1).map_err(db)? != "assistant" {
        return Err(super::failure(
            StorageErrorKind::InvalidData,
            "regeneration target is not an assistant",
        ));
    }

    drop(rows);
    drop(statement);

    let previous = read_context(transaction, id, sequence).await?;
    let user_message = previous
        .iter()
        .rev()
        .find(|message| message.role == MessageRole::User)
        .cloned()
        .ok_or_else(|| super::failure(StorageErrorKind::InvalidData, "missing user context"))?;

    let (context, context_report) =
        select_context(&previous, conversation.generation.limits, overhead)
            .map_err(|error| StorageError::new(StorageErrorKind::ContextTooLarge, error))?;

    Ok(RegenerationContext {
        sequence,
        user_message,
        context,
        context_report,
    })
}

pub(super) async fn regenerate(
    connection: &mut Connection,
    id: ConversationId,
    target: MessageId,
    overhead: u64,
) -> Result<PreparedTurn> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .await
        .map_err(db)?;

    ensure_idle(&transaction, id).await?;

    let conversation = read_conversation(&transaction, id).await?;

    let RegenerationContext {
        sequence,
        user_message,
        context,
        context_report,
    } = read_regeneration_context(&transaction, id, target, &conversation, overhead).await?;

    let generation = serde_json::to_string(&conversation.generation).map_err(super::invalid)?;

    transaction
        .execute(
            "UPDATE messages SET content='',status='streaming',outcome=NULL, \
             failure=NULL,thinking_duration_ms=NULL,generation=?1,omitted_context_messages=?2 \
             WHERE id=?3",
            params![
                generation,
                i64::try_from(context_report.omitted_messages).map_err(super::invalid)?,
                as_i64(target.0)?,
            ],
        )
        .await
        .map_err(db)?;

    transaction
        .execute(
            "DELETE FROM assistant_traces WHERE assistant_message_id=?1",
            [as_i64(target.0)?],
        )
        .await
        .map_err(db)?;

    if conversation.mode == ConversationMode::Agent {
        transaction
            .execute(
                "UPDATE agent_runs SET status='running',started_at=?1,finished_at=NULL \
                 WHERE assistant_message_id=?2",
                params![super::now()?, as_i64(target.0)?],
            )
            .await
            .map_err(db)?;
    }

    transaction
        .execute(
            "UPDATE conversations SET updated_at=?1 WHERE id=?2",
            params![super::now()?, as_i64(id.0)?],
        )
        .await
        .map_err(db)?;

    let assistant_message = Message {
        id: target,
        conversation_id: id,
        role: MessageRole::Assistant,
        command_id: None,
        content: String::new(),
        status: MessageStatus::Streaming,
        attachments: Vec::new(),
        generation_outcome: None,
        failure: None,
        assistant_trace: AssistantTrace::default(),
    };

    transaction.commit().await.map_err(db)?;

    Ok(PreparedTurn {
        conversation,
        user_message,
        assistant_message,
        context,
        agent_run_id: None,
        user_sequence: MessageSequence(sequence.saturating_sub(1)),
        assistant_sequence: MessageSequence(sequence),
        context_report,
    })
}

pub(super) async fn retry(
    connection: &mut Connection,
    id: ConversationId,
    target: MessageId,
    generation: GenerationConfig,
    overhead: u64,
) -> Result<PreparedTurn> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .await
        .map_err(db)?;

    ensure_idle(&transaction, id).await?;

    let mut conversation = read_conversation(&transaction, id).await?;

    let mut statement = transaction
        .prepare("SELECT sequence,role,status FROM messages WHERE conversation_id=?1 AND id=?2")
        .await
        .map_err(db)?;

    let mut rows = statement
        .query(params![as_i64(id.0)?, as_i64(target.0)?])
        .await
        .map_err(db)?;

    let Some(row) = rows.next().await.map_err(db)? else {
        return Err(super::failure(
            StorageErrorKind::NotFound,
            "message does not exist",
        ));
    };

    let target_sequence: i64 = row.get(0).map_err(db)?;

    if row.get::<String>(1).map_err(db)? != "assistant"
        || row.get::<String>(2).map_err(db)? != "failed"
    {
        return Err(super::failure(
            StorageErrorKind::InvalidData,
            "retry target is not a failed assistant",
        ));
    }

    drop(rows);
    drop(statement);

    let previous = read_context(&transaction, id, target_sequence).await?;
    let user_message = previous
        .iter()
        .rev()
        .find(|message| message.role == MessageRole::User)
        .cloned()
        .ok_or_else(|| super::failure(StorageErrorKind::InvalidData, "missing user context"))?;

    let user_sequence = scalar_i64(
        &transaction,
        "SELECT sequence FROM messages WHERE id=?1",
        [as_i64(user_message.id.0)?],
    )
    .await?;

    let (context, context_report) = select_context(&previous, generation.limits, overhead)
        .map_err(|error| StorageError::new(StorageErrorKind::ContextTooLarge, error))?;

    let generation_json = serde_json::to_string(&generation).map_err(super::invalid)?;
    let timestamp = super::now()?;

    transaction
        .execute(
            "UPDATE conversations SET generation=?1,updated_at=?2 WHERE id=?3",
            params![generation_json.as_str(), timestamp, as_i64(id.0)?],
        )
        .await
        .map_err(db)?;

    conversation.generation = generation;

    let sequence = scalar_i64(
        &transaction,
        "SELECT COALESCE(MAX(sequence)+1,0) FROM messages WHERE conversation_id=?1",
        [as_i64(id.0)?],
    )
    .await?;

    let assistant_message = insert_assistant(
        &transaction,
        id,
        sequence,
        &generation_json,
        timestamp,
        context_report.omitted_messages,
    )
    .await?;

    let agent_run_id =
        insert_agent_run(&transaction, &conversation, &assistant_message, timestamp).await?;

    transaction.commit().await.map_err(db)?;
    Ok(PreparedTurn {
        conversation,
        user_message,
        assistant_message,
        context,
        agent_run_id,
        user_sequence: MessageSequence(user_sequence),
        assistant_sequence: MessageSequence(sequence),
        context_report,
    })
}

struct UserMessageInput<'a> {
    conversation_id: ConversationId,
    sequence: i64,
    content: String,
    attachments: Vec<Attachment>,
    command_id: Option<CommandId>,
    generation: &'a str,
    timestamp: i64,
}

async fn insert_user(connection: &Connection, input: UserMessageInput<'_>) -> Result<Message> {
    connection
        .execute(
            "INSERT INTO messages( \
             conversation_id,sequence,role,content,status,generation,command_id,created_at \
             ) VALUES (?1,?2,'user',?3,'complete',?4,?5,?6)",
            params![
                as_i64(input.conversation_id.0)?,
                input.sequence,
                input.content.as_str(),
                input.generation,
                input.command_id.as_ref().map(CommandId::as_str),
                input.timestamp,
            ],
        )
        .await
        .map_err(db)?;

    let id = MessageId(as_u64(connection.last_insert_rowid())?);

    for (position, attachment) in input.attachments.iter().enumerate() {
        connection
            .execute(
                "INSERT INTO attachments( \
                 message_id,position,name,source_path,mime_type,byte_size,managed \
                 ) VALUES (?1,?2,?3,?4,?5,?6,?7)",
                params![
                    as_i64(id.0)?,
                    i64::try_from(position).map_err(super::invalid)?,
                    attachment.name.as_str(),
                    super::records::encode_path(&attachment.path),
                    attachment.mime_type.as_str(),
                    i64::try_from(attachment.byte_size).map_err(super::invalid)?,
                    attachment.managed,
                ],
            )
            .await
            .map_err(db)?;
    }

    Ok(Message {
        id,
        conversation_id: input.conversation_id,
        role: MessageRole::User,
        command_id: input.command_id,
        content: input.content,
        status: MessageStatus::Complete,
        attachments: input.attachments,
        generation_outcome: None,
        failure: None,
        assistant_trace: AssistantTrace::default(),
    })
}

async fn insert_assistant(
    connection: &Connection,
    conversation_id: ConversationId,
    sequence: i64,
    generation: &str,
    timestamp: i64,
    omitted: usize,
) -> Result<Message> {
    connection
        .execute(
            "INSERT INTO messages( \
             conversation_id,sequence,role,content,status,generation, \
             omitted_context_messages,created_at \
             ) VALUES (?1,?2,'assistant','','streaming',?3,?4,?5)",
            params![
                as_i64(conversation_id.0)?,
                sequence,
                generation,
                i64::try_from(omitted).map_err(super::invalid)?,
                timestamp,
            ],
        )
        .await
        .map_err(db)?;

    Ok(Message {
        id: MessageId(as_u64(connection.last_insert_rowid())?),
        conversation_id,
        role: MessageRole::Assistant,
        command_id: None,
        content: String::new(),
        status: MessageStatus::Streaming,
        attachments: Vec::new(),
        generation_outcome: None,
        failure: None,
        assistant_trace: AssistantTrace::default(),
    })
}

async fn insert_agent_run(
    connection: &Connection,
    conversation: &Conversation,
    assistant: &Message,
    timestamp: i64,
) -> Result<Option<AgentRunId>> {
    if conversation.mode != ConversationMode::Agent {
        return Ok(None);
    }

    connection
        .execute(
            "INSERT INTO agent_runs( \
             conversation_id,assistant_message_id,status,started_at \
             ) VALUES (?1,?2,'running',?3)",
            params![
                as_i64(conversation.id.0)?,
                as_i64(assistant.id.0)?,
                timestamp,
            ],
        )
        .await
        .map_err(db)?;

    Ok(Some(AgentRunId(as_u64(connection.last_insert_rowid())?)))
}
