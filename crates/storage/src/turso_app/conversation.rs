use super::{
    Arc, AssistantTrace, BeginTurn, ConversationId, ConversationPage, ConversationSearchResult,
    ConversationStore, ConversationSummary, GenerationConfig, Message, MessageId, MessagePage,
    MessageRole, MessageSequence, MessageStatus, OptionalExtension, Ordering, PreparedTurn, Result,
    SCHEMA, SCHEMA_VERSION, StorageErrorKind, StorageFuture, Timestamp, TransactionBehavior,
    TursoAppStore, as_i64, as_u64, db, decode_optional_path, highlights, migration, params,
    parse_mode, reading, run_status, schema_version, status_name, trace, turns,
};

impl ConversationStore for TursoAppStore {
    fn initialize(&self) -> StorageFuture<()> {
        let path = Arc::clone(&self.path);
        let initialized = Arc::clone(&self.initialized);

        Box::pin(async move {
            if initialized.load(Ordering::Acquire) {
                return Ok(());
            }

            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(super::unavailable)?;
            }

            repair_legacy_message_index(&path)?;
            let mut connection = Self::connect_path(&path).await?;
            connection.execute_batch(SCHEMA).await.map_err(db)?;

            let version = schema_version(&connection).await?;

            if version > SCHEMA_VERSION {
                return Err(super::failure(
                    StorageErrorKind::UnsupportedVersion,
                    "app database was created by a newer Magenta",
                ));
            }

            if version == 1 {
                migration::migrate_v1_to_v2(&mut connection).await?;
            }
            if (1..=2).contains(&version) {
                migration::migrate_v2_to_v3(&connection).await?;
            }

            connection
                .execute(
                    "INSERT INTO _magenta_schema(component, version) \
                     VALUES ('app', ?1) \
                     ON CONFLICT(component) DO UPDATE \
                     SET version = excluded.version",
                    [SCHEMA_VERSION],
                )
                .await
                .map_err(db)?;

            let timestamp = super::now()?;

            connection
                .execute(
                    "UPDATE messages SET status = 'stopped' WHERE status = 'streaming'",
                    (),
                )
                .await
                .map_err(db)?;

            connection
                .execute(
                    "UPDATE agent_runs \
                     SET status = 'stopped', finished_at = ?1 \
                     WHERE status = 'running'",
                    [timestamp],
                )
                .await
                .map_err(db)?;

            initialized.store(true, Ordering::Release);
            Ok(())
        })
    }

    fn summaries(&self) -> StorageFuture<Vec<ConversationSummary>> {
        self.run(async |connection| {
            let mut statement = connection
                .prepare(
                    "SELECT id, title, \
                     COALESCE((SELECT substr(trim(content), 1, 240) \
                     FROM messages \
                     WHERE conversation_id = conversations.id \
                       AND role = 'user' \
                       AND trim(content) <> '' \
                     ORDER BY sequence DESC LIMIT 1), ''), \
                     pinned, mode, workspace_root, created_at, updated_at, generation \
                     FROM conversations \
                     ORDER BY updated_at DESC, id DESC",
                )
                .await
                .map_err(db)?;

            let mut rows = statement.query(()).await.map_err(db)?;

            let mut result = Vec::new();

            while let Some(row) = rows.next().await.map_err(db)? {
                let generation: String = row.get(8).map_err(db)?;
                let generation: GenerationConfig =
                    serde_json::from_str(&generation).map_err(super::invalid)?;

                result.push(ConversationSummary {
                    id: ConversationId(as_u64(row.get(0).map_err(db)?)?),
                    title: row.get(1).map_err(db)?,
                    preview: row.get(2).map_err(db)?,
                    pinned: row.get::<i64>(3).map_err(db)? != 0,
                    mode: parse_mode(&row.get::<String>(4).map_err(db)?)?,
                    workspace_root: decode_optional_path(row.get(5).map_err(db)?)?,
                    created_at: Timestamp(row.get(6).map_err(db)?),
                    updated_at: Timestamp(row.get(7).map_err(db)?),
                    provider: generation.provider,
                });
            }

            Ok(result)
        })
    }

    fn search(&self, query: String, limit: usize) -> StorageFuture<Vec<ConversationSearchResult>> {
        self.run(async move |connection| {
            let query = query.trim().to_lowercase();

            if query.is_empty() {
                return Ok(Vec::new());
            }

            let pattern = format!("%{query}%");
            let mut statement = connection
                .prepare(
                    "SELECT c.id, c.title, c.updated_at, m.id, m.sequence, \
                     COALESCE(substr(trim(m.content), 1, 240), '') \
                     FROM conversations c \
                     LEFT JOIN messages m ON m.id = ( \
                         SELECT id FROM messages \
                         WHERE conversation_id = c.id \
                           AND lower(content) LIKE ?1 \
                         ORDER BY sequence DESC LIMIT 1 \
                     ) \
                     WHERE lower(c.title) LIKE ?1 OR m.id IS NOT NULL \
                     ORDER BY c.updated_at DESC LIMIT ?2",
                )
                .await
                .map_err(db)?;

            let mut rows = statement
                .query(params![
                    pattern,
                    i64::try_from(limit.min(100)).map_err(super::invalid)?,
                ])
                .await
                .map_err(db)?;

            let mut result = Vec::new();

            while let Some(row) = rows.next().await.map_err(db)? {
                let title: String = row.get(1).map_err(db)?;
                let snippet: String = row.get(5).map_err(db)?;

                result.push(ConversationSearchResult {
                    conversation_id: ConversationId(as_u64(row.get(0).map_err(db)?)?),
                    message_id: row
                        .get::<Option<i64>>(3)
                        .map_err(db)?
                        .map(as_u64)
                        .transpose()?
                        .map(MessageId),
                    message_sequence: row.get::<Option<i64>>(4).map_err(db)?.map(MessageSequence),
                    title_highlights: highlights(&title, &query),
                    snippet_highlights: highlights(&snippet, &query),
                    title,
                    snippet,
                    updated_at: Timestamp(row.get(2).map_err(db)?),
                });
            }

            Ok(result)
        })
    }

    fn load(&self, id: ConversationId) -> StorageFuture<ConversationPage> {
        self.run(async move |connection| {
            let conversation = reading::read_conversation(&connection, id).await?;

            let page = reading::read_page(&connection, id, None, None).await?;

            Ok(ConversationPage { conversation, page })
        })
    }

    fn load_around(
        &self,
        id: ConversationId,
        sequence: MessageSequence,
    ) -> StorageFuture<ConversationPage> {
        self.run(async move |connection| {
            let conversation = reading::read_conversation(&connection, id).await?;
            let first = sequence.0.saturating_sub(24).max(0);

            let page =
                reading::read_range(&connection, id, first, sequence.0.saturating_add(25)).await?;

            if page.messages.is_empty() {
                return Err(super::failure(
                    StorageErrorKind::NotFound,
                    "search result message does not exist",
                ));
            }

            Ok(ConversationPage { conversation, page })
        })
    }

    fn earlier(&self, id: ConversationId, before: MessageSequence) -> StorageFuture<MessagePage> {
        self.run(async move |connection| {
            reading::read_page(&connection, id, Some(before), None).await
        })
    }

    fn later(&self, id: ConversationId, after: MessageSequence) -> StorageFuture<MessagePage> {
        self.run(async move |connection| {
            reading::read_page(&connection, id, None, Some(after)).await
        })
    }

    fn begin_turn(&self, input: BeginTurn) -> StorageFuture<PreparedTurn> {
        let this = self.clone();
        Box::pin(async move {
            if !this.initialized.load(Ordering::Acquire) {
                return Err(super::failure(
                    StorageErrorKind::Unavailable,
                    "storage has not been initialized",
                ));
            }
            let imported = super::attachments::import(&this.attachments_path, &input.attachments)?;

            let result = turns::begin_turn(
                &mut Self::connect_path(&this.path).await?,
                input,
                imported.clone(),
            )
            .await;

            if result.is_err() {
                super::attachments::remove_managed(&this.attachments_path, &imported);
            }

            result
        })
    }

    fn begin_regeneration(
        &self,
        id: ConversationId,
        target: MessageId,
        request_overhead_tokens: u64,
    ) -> StorageFuture<PreparedTurn> {
        self.run(async move |mut connection| {
            turns::regenerate(&mut connection, id, target, request_overhead_tokens).await
        })
    }

    fn begin_retry(
        &self,
        id: ConversationId,
        target: MessageId,
        generation: GenerationConfig,
        request_overhead_tokens: u64,
    ) -> StorageFuture<PreparedTurn> {
        self.run(async move |mut connection| {
            turns::retry(
                &mut connection,
                id,
                target,
                generation,
                request_overhead_tokens,
            )
            .await
        })
    }

    fn command_for_response(
        &self,
        conversation_id: ConversationId,
        assistant_message_id: MessageId,
    ) -> StorageFuture<Option<magenta_core::CommandId>> {
        self.run(async move |connection| {
            let mut statement = connection
                .prepare(
                    "SELECT m.command_id FROM messages m JOIN messages a \
                     ON a.conversation_id=m.conversation_id \
                     AND a.role='assistant' AND a.id=?2 \
                     WHERE m.conversation_id=?1 AND m.role='user' AND m.sequence<a.sequence \
                     ORDER BY m.sequence DESC LIMIT 1",
                )
                .await
                .map_err(db)?;
            let mut rows = statement
                .query(params![
                    as_i64(conversation_id.0)?,
                    as_i64(assistant_message_id.0)?
                ])
                .await
                .map_err(db)?;
            let value = rows
                .next()
                .await
                .map_err(db)?
                .map(|row| row.get::<Option<String>>(0))
                .transpose()
                .map_err(db)?;
            Ok(value.flatten().map(magenta_core::CommandId::new))
        })
    }

    fn finalize(&self, message: Message) -> StorageFuture<()> {
        self.run(async move |mut connection| {
            if message.role != MessageRole::Assistant || message.status == MessageStatus::Streaming
            {
                return Err(super::failure(
                    StorageErrorKind::InvalidData,
                    "only terminal assistant messages can be finalized",
                ));
            }

            let transaction = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .await
                .map_err(db)?;

            let outcome = message
                .generation_outcome
                .as_ref()
                .map(serde_json::to_string)
                .transpose()
                .map_err(super::invalid)?;

            let failure = message
                .failure
                .as_ref()
                .map(serde_json::to_string)
                .transpose()
                .map_err(super::invalid)?;

            let changed = transaction
                .execute(
                    "UPDATE messages SET content=?1,status=?2,outcome=?3,failure=?4,thinking_duration_ms=?5 \
                     WHERE id=?6 AND conversation_id=?7 AND status='streaming'",
                    params![
                        message.content,
                        status_name(message.status),
                        outcome,
                        failure,
                        message.assistant_trace.thinking_duration_ms.map(as_i64).transpose()?,
                        as_i64(message.id.0)?,
                        as_i64(message.conversation_id.0)?,
                    ],
                )
                .await
                .map_err(db)?;

            if changed != 1 {
                return Err(super::failure(
                    StorageErrorKind::Conflict,
                    "message is no longer streaming",
                ));
            }

            trace::replace_trace(&transaction, message.id, &message.assistant_trace).await?;

            let timestamp = super::now()?;

            transaction
                .execute(
                    "UPDATE agent_runs SET status=?1,finished_at=?2 \
                     WHERE assistant_message_id=?3 AND status='running'",
                    params![run_status(message.status), timestamp, as_i64(message.id.0)?,],
                )
                .await
                .map_err(db)?;

            transaction
                .execute(
                    "UPDATE conversations SET updated_at=?1 WHERE id=?2",
                    params![timestamp, as_i64(message.conversation_id.0)?],
                )
                .await
                .map_err(db)?;

            transaction.commit().await.map_err(db)
        })
    }

    fn delete(&self, id: ConversationId) -> StorageFuture<()> {
        let this = self.clone();
        Box::pin(async move {
            if !this.initialized.load(Ordering::Acquire) {
                return Err(super::failure(
                    StorageErrorKind::Unavailable,
                    "storage has not been initialized",
                ));
            }

            let mut connection = Self::connect_path(&this.path).await?;
            let managed = reading::read_managed_attachments(&connection, id).await?;

            let transaction = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .await
                .map_err(db)?;

            let changed = transaction
                .execute("DELETE FROM conversations WHERE id=?1", [as_i64(id.0)?])
                .await
                .map_err(db)?;

            if changed == 0 {
                return Err(super::failure(
                    StorageErrorKind::NotFound,
                    "conversation does not exist",
                ));
            }

            transaction.commit().await.map_err(db)?;
            super::attachments::remove_managed(&this.attachments_path, &managed);
            Ok(())
        })
    }

    fn rename(&self, id: ConversationId, title: String) -> StorageFuture<()> {
        self.run(async move |connection| {
            let changed = connection
                .execute(
                    "UPDATE conversations SET title=?1 WHERE id=?2",
                    params![title, as_i64(id.0)?],
                )
                .await
                .map_err(db)?;

            if changed == 0 {
                return Err(super::failure(
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
        self.run(async move |connection| {
            Ok(connection
                .execute(
                    "UPDATE conversations SET title=?1 WHERE id=?2 AND title=?3",
                    params![title, as_i64(id.0)?, current],
                )
                .await
                .map_err(db)?
                == 1)
        })
    }

    fn set_pinned(&self, id: ConversationId, pinned: bool) -> StorageFuture<()> {
        self.run(async move |connection| {
            let changed = connection
                .execute(
                    "UPDATE conversations SET pinned=?1 WHERE id=?2",
                    params![pinned, as_i64(id.0)?],
                )
                .await
                .map_err(db)?;

            if changed == 0 {
                return Err(super::failure(
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
        self.run(async move |mut connection| {
            let transaction = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .await
                .map_err(db)?;
            let duration = trace.thinking_duration_ms.map(as_i64).transpose()?;
            let changed = transaction
                .execute(
                    "UPDATE messages SET thinking_duration_ms=?1 WHERE id=?2",
                    params![duration, as_i64(message_id.0)?],
                )
                .await
                .map_err(db)?;
            if changed == 0 {
                return Err(super::failure(
                    StorageErrorKind::NotFound,
                    "assistant message does not exist",
                ));
            }
            trace::replace_trace(&transaction, message_id, &trace).await?;
            transaction.commit().await.map_err(db)
        })
    }
}

fn repair_legacy_message_index(path: &std::path::Path) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }

    let mut connection = rusqlite::Connection::open(path).map_err(crate::database_error)?;
    connection
        .pragma_update(None, "writable_schema", true)
        .map_err(crate::database_error)?;

    let table_sql = connection
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name='messages'",
            (),
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(crate::database_error)?;
    let Some(table_sql) = table_sql else {
        return Ok(());
    };

    let has_inline_unique = table_sql.to_ascii_uppercase().contains("UNIQUE");
    let orphaned_index = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master \
             WHERE type='index' AND name='sqlite_autoindex_messages_1' \
               AND tbl_name='messages' AND sql IS NULL)",
            (),
            |row| row.get::<_, i64>(0),
        )
        .map_err(crate::database_error)?
        != 0;

    if has_inline_unique || !orphaned_index {
        return Ok(());
    }

    let duplicate_sequence = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM messages \
             GROUP BY conversation_id, sequence HAVING COUNT(*) > 1)",
            (),
            |row| row.get::<_, i64>(0),
        )
        .map_err(crate::database_error)?
        != 0;
    if duplicate_sequence {
        return Err(super::failure(
            StorageErrorKind::InvalidData,
            "cannot repair duplicate message sequences",
        ));
    }

    let schema_version = connection
        .pragma_query_value(None, "schema_version", |row| row.get::<_, i64>(0))
        .map_err(crate::database_error)?;
    let transaction = connection
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(crate::database_error)?;
    transaction
        .execute(
            "DELETE FROM sqlite_master WHERE type='index' \
             AND name='sqlite_autoindex_messages_1' AND tbl_name='messages'",
            (),
        )
        .map_err(crate::database_error)?;
    transaction
        .execute_batch(&format!(
            "PRAGMA schema_version = {}",
            schema_version.saturating_add(1)
        ))
        .map_err(crate::database_error)?;
    transaction.commit().map_err(crate::database_error)?;

    connection
        .execute(
            "CREATE UNIQUE INDEX IF NOT EXISTS message_conversation_sequence \
             ON messages(conversation_id, sequence)",
            (),
        )
        .map_err(crate::database_error)?;
    Ok(())
}
