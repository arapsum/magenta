//! Local Turso implementation of Magenta's primary application store.

use std::{
    future::Future,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use magenta_core::{
    AgentActivity, AgentActivityKind, AgentActivityRecord, AgentRunId, Attachment, BeginTurn,
    Conversation, ConversationId, ConversationMode, ConversationPage, ConversationSearchResult,
    ConversationStore, ConversationSummary, GenerationConfig, Message, MessageId, MessagePage,
    MessageRole, MessageSequence, MessageStatus, PreparedTurn, Project, ProjectStore, StorageError,
    StorageErrorKind, StorageFuture, StoredMessage, Timestamp, select_context,
};
use turso::{
    Connection, Row, params,
    transaction::{Transaction, TransactionBehavior},
};

type Result<T> = std::result::Result<T, StorageError>;
const SCHEMA_VERSION: i64 = 1;
const PAGE_SIZE: usize = 50;

const SCHEMA: &str = r"
CREATE TABLE IF NOT EXISTS _magenta_schema(component TEXT PRIMARY KEY, version INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS conversations (
 id INTEGER PRIMARY KEY AUTOINCREMENT, title TEXT NOT NULL, generation TEXT NOT NULL,
 mode TEXT NOT NULL, workspace_root BLOB, pinned INTEGER NOT NULL DEFAULT 0,
 created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL
);
-- Turso 0.4.4 can corrupt a secondary index when an indexed recency value is
-- updated in the same transaction. These datasets are small and ordering does
-- not require an index, so remove indexes over mutable recency columns.
DROP INDEX IF EXISTS conversation_recency;
CREATE TABLE IF NOT EXISTS messages (
 id INTEGER PRIMARY KEY AUTOINCREMENT, conversation_id INTEGER NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
 sequence INTEGER NOT NULL, role TEXT NOT NULL, content TEXT NOT NULL, status TEXT NOT NULL,
 generation TEXT NOT NULL, outcome TEXT, failure TEXT, omitted_context_messages INTEGER NOT NULL DEFAULT 0,
 created_at INTEGER NOT NULL, UNIQUE(conversation_id, sequence)
);
CREATE UNIQUE INDEX IF NOT EXISTS one_stream_per_conversation ON messages(conversation_id) WHERE status = 'streaming';
CREATE INDEX IF NOT EXISTS message_conversation_order ON messages(conversation_id, sequence);
CREATE TABLE IF NOT EXISTS projects (
 root BLOB PRIMARY KEY, name TEXT NOT NULL, added_at INTEGER NOT NULL, last_opened_at INTEGER NOT NULL
);
DROP INDEX IF EXISTS project_recency;
CREATE TABLE IF NOT EXISTS agent_runs (
 id INTEGER PRIMARY KEY AUTOINCREMENT, conversation_id INTEGER NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
 assistant_message_id INTEGER NOT NULL UNIQUE REFERENCES messages(id) ON DELETE CASCADE,
 status TEXT NOT NULL, started_at INTEGER NOT NULL, finished_at INTEGER
);
CREATE TABLE IF NOT EXISTS agent_activities (
 id INTEGER PRIMARY KEY AUTOINCREMENT, run_id INTEGER NOT NULL REFERENCES agent_runs(id) ON DELETE CASCADE,
 sequence INTEGER NOT NULL, kind TEXT NOT NULL, call_id TEXT NOT NULL, tool_name TEXT NOT NULL,
 status TEXT NOT NULL, summary TEXT NOT NULL, detail TEXT NOT NULL, created_at INTEGER NOT NULL,
 UNIQUE(run_id, sequence)
);
CREATE INDEX IF NOT EXISTS agent_activity_order ON agent_activities(run_id, sequence);
CREATE TABLE IF NOT EXISTS attachments (
 message_id INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE, position INTEGER NOT NULL,
 name TEXT NOT NULL, source_path BLOB NOT NULL, mime_type TEXT NOT NULL, byte_size INTEGER NOT NULL,
 managed INTEGER NOT NULL, PRIMARY KEY(message_id, position)
);
";

#[derive(Clone)]
pub struct TursoAppStore {
    path: Arc<PathBuf>,
    attachments_path: Arc<PathBuf>,
    initialized: Arc<AtomicBool>,
}

impl TursoAppStore {
    #[must_use]
    pub fn new(path: PathBuf) -> Self {
        Self {
            attachments_path: Arc::new(super::attachment_directory(&path)),
            path: Arc::new(path),
            initialized: Arc::new(AtomicBool::new(false)),
        }
    }

    async fn connect_path(path: &std::path::Path) -> Result<Connection> {
        let database = turso::Builder::new_local(&path.to_string_lossy())
            .build()
            .await
            .map_err(db)?;

        let connection = database.connect().map_err(db)?;

        connection
            .execute("PRAGMA foreign_keys = ON", ())
            .await
            .map_err(db)?;
        Ok(connection)
    }

    fn run<T, F, Fut>(&self, operation: F) -> StorageFuture<T>
    where
        T: Send + 'static,
        F: FnOnce(Connection) -> Fut + Send + 'static,
        Fut: Future<Output = Result<T>> + Send + 'static,
    {
        let path = Arc::clone(&self.path);
        let initialized = Arc::clone(&self.initialized);

        Box::pin(async move {
            if !initialized.load(Ordering::Acquire) {
                return Err(super::failure(
                    StorageErrorKind::Unavailable,
                    "storage has not been initialized",
                ));
            }
            operation(Self::connect_path(&path).await?).await
        })
    }
}

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

            let connection = Self::connect_path(&path).await?;
            connection.execute_batch(SCHEMA).await.map_err(db)?;

            let version = schema_version(&connection).await?;

            if version > SCHEMA_VERSION {
                return Err(super::failure(
                    StorageErrorKind::UnsupportedVersion,
                    "app database was created by a newer Magenta",
                ));
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
            let conversation = read_conversation(&connection, id).await?;

            let page = read_page(&connection, id, None, None).await?;

            Ok(ConversationPage { conversation, page })
        })
    }

    fn load_around(
        &self,
        id: ConversationId,
        sequence: MessageSequence,
    ) -> StorageFuture<ConversationPage> {
        self.run(async move |connection| {
            let conversation = read_conversation(&connection, id).await?;
            let first = sequence.0.saturating_sub(24).max(0);

            let page = read_range(&connection, id, first, sequence.0.saturating_add(25)).await?;

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
        self.run(async move |connection| read_page(&connection, id, Some(before), None).await)
    }

    fn later(&self, id: ConversationId, after: MessageSequence) -> StorageFuture<MessagePage> {
        self.run(async move |connection| read_page(&connection, id, None, Some(after)).await)
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

            let result = begin_turn(
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
            regenerate(&mut connection, id, target, request_overhead_tokens).await
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
            retry(
                &mut connection,
                id,
                target,
                generation,
                request_overhead_tokens,
            )
            .await
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
                    "UPDATE messages SET content=?1,status=?2,outcome=?3,failure=?4 \
                     WHERE id=?5 AND conversation_id=?6 AND status='streaming'",
                    params![
                        message.content,
                        status_name(message.status),
                        outcome,
                        failure,
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
            let managed = read_managed_attachments(&connection, id).await?;

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

    fn append_agent_activity(&self, record: AgentActivityRecord) -> StorageFuture<()> {
        self.run(async move |mut connection| {
            let transaction = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .await
                .map_err(db)?;

            let sequence = scalar_i64(
                &transaction,
                "SELECT COALESCE(MAX(sequence)+1,0) \
                 FROM agent_activities WHERE run_id=?1",
                [as_i64(record.run_id.0)?],
            )
            .await?;

            transaction
                .execute(
                    "INSERT INTO agent_activities( \
                     run_id,sequence,kind,call_id,tool_name,status,summary,detail,created_at \
                     ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                    params![
                        as_i64(record.run_id.0)?,
                        sequence,
                        activity_kind(&record.activity.kind),
                        record.activity.call_id,
                        record.activity.tool_name,
                        record.activity.status,
                        record.activity.summary,
                        record.activity.detail,
                        super::now()?,
                    ],
                )
                .await
                .map_err(db)?;

            transaction.commit().await.map_err(db)
        })
    }
}

impl ProjectStore for TursoAppStore {
    fn projects(&self) -> StorageFuture<Vec<Project>> {
        self.run(async |connection| {
            let mut statement = connection
                .prepare(
                    "SELECT name,root,added_at,last_opened_at \
                     FROM projects \
                     ORDER BY last_opened_at DESC,name COLLATE NOCASE",
                )
                .await
                .map_err(db)?;
            let mut rows = statement.query(()).await.map_err(db)?;
            let mut result = Vec::new();

            while let Some(row) = rows.next().await.map_err(db)? {
                result.push(Project {
                    name: row.get(0).map_err(db)?,
                    root: super::records::decode_path(row.get(1).map_err(db)?)?,
                    added_at: Timestamp(row.get(2).map_err(db)?),
                    last_opened_at: Timestamp(row.get(3).map_err(db)?),
                });
            }

            Ok(result)
        })
    }

    fn upsert_project(&self, project: Project) -> StorageFuture<()> {
        self.run(async move |connection| {
            connection
                .execute(
                    "INSERT INTO projects(root,name,added_at,last_opened_at) \
                     VALUES (?1,?2,?3,?4) \
                     ON CONFLICT(root) DO UPDATE SET \
                     name=excluded.name,last_opened_at=excluded.last_opened_at",
                    params![
                        super::records::encode_path(&project.root),
                        project.name,
                        project.added_at.0,
                        project.last_opened_at.0,
                    ],
                )
                .await
                .map_err(db)?;

            Ok(())
        })
    }

    fn remove_project(&self, root: PathBuf) -> StorageFuture<()> {
        self.run(async move |connection| {
            connection
                .execute(
                    "DELETE FROM projects WHERE root=?1",
                    [super::records::encode_path(&root)],
                )
                .await
                .map_err(db)?;

            Ok(())
        })
    }
}

async fn begin_turn(
    connection: &mut Connection,
    input: BeginTurn,
    attachments: Vec<Attachment>,
) -> Result<PreparedTurn> {
    let BeginTurn {
        conversation_id,
        title,
        prompt,
        attachments: _,
        generation,
        mode,
        workspace_root,
        request_overhead_tokens,
    } = input;

    if prompt.trim().is_empty() && attachments.is_empty() {
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
        conversation.id,
        sequence,
        prompt,
        attachments,
        &generation_json,
        timestamp,
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

async fn regenerate(
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

    let previous = read_context(&transaction, id, sequence).await?;
    let user_message = previous
        .iter()
        .rev()
        .find(|message| message.role == MessageRole::User)
        .cloned()
        .ok_or_else(|| super::failure(StorageErrorKind::InvalidData, "missing user context"))?;

    let (context, context_report) =
        select_context(&previous, conversation.generation.limits, overhead)
            .map_err(|error| StorageError::new(StorageErrorKind::ContextTooLarge, error))?;

    let generation = serde_json::to_string(&conversation.generation).map_err(super::invalid)?;

    transaction
        .execute(
            "UPDATE messages SET content='',status='streaming',outcome=NULL, \
             failure=NULL,generation=?1,omitted_context_messages=?2 WHERE id=?3",
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
            "UPDATE conversations SET updated_at=?1 WHERE id=?2",
            params![super::now()?, as_i64(id.0)?],
        )
        .await
        .map_err(db)?;

    let assistant_message = Message {
        id: target,
        conversation_id: id,
        role: MessageRole::Assistant,
        content: String::new(),
        status: MessageStatus::Streaming,
        attachments: Vec::new(),
        generation_outcome: None,
        failure: None,
        agent_activities: Vec::new(),
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

async fn retry(
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

async fn insert_user(
    connection: &Connection,
    conversation_id: ConversationId,
    sequence: i64,
    content: String,
    attachments: Vec<Attachment>,
    generation: &str,
    timestamp: i64,
) -> Result<Message> {
    connection
        .execute(
            "INSERT INTO messages( \
             conversation_id,sequence,role,content,status,generation,created_at \
             ) VALUES (?1,?2,'user',?3,'complete',?4,?5)",
            params![
                as_i64(conversation_id.0)?,
                sequence,
                content.as_str(),
                generation,
                timestamp,
            ],
        )
        .await
        .map_err(db)?;

    let id = MessageId(as_u64(connection.last_insert_rowid())?);

    for (position, attachment) in attachments.iter().enumerate() {
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
        conversation_id,
        role: MessageRole::User,
        content,
        status: MessageStatus::Complete,
        attachments,
        generation_outcome: None,
        failure: None,
        agent_activities: Vec::new(),
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
        content: String::new(),
        status: MessageStatus::Streaming,
        attachments: Vec::new(),
        generation_outcome: None,
        failure: None,
        agent_activities: Vec::new(),
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

async fn ensure_idle(connection: &Connection, id: ConversationId) -> Result<()> {
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

async fn read_conversation(connection: &Connection, id: ConversationId) -> Result<Conversation> {
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
        created_at: row.get(8).map_err(db)?,
        omitted: row.get(9).map_err(db)?,
    })
}

async fn read_page(
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
                 created_at,omitted_context_messages \
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
                 created_at,omitted_context_messages \
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

async fn read_range(
    connection: &Connection,
    id: ConversationId,
    first: i64,
    last: i64,
) -> Result<MessagePage> {
    let mut statement = connection
        .prepare(
            "SELECT id,sequence,role,content,status,generation,outcome,failure, \
             created_at,omitted_context_messages \
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

async fn read_context(
    connection: &Connection,
    id: ConversationId,
    before: i64,
) -> Result<Vec<Message>> {
    let mut statement = connection
        .prepare(
            "SELECT id,sequence,role,content,status,generation,outcome,failure, \
             created_at,omitted_context_messages \
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
    let activities = read_activities(connection, id).await?;

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
        agent_activities: activities.clone(),
    };

    Ok(StoredMessage {
        message,
        sequence: MessageSequence(raw.sequence),
        created_at: Timestamp(raw.created_at),
        generation,
        agent_activities: activities,
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

async fn read_managed_attachments(
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

async fn read_activities(connection: &Connection, id: MessageId) -> Result<Vec<AgentActivity>> {
    let mut statement = connection
        .prepare(
            "SELECT a.kind,a.call_id,a.tool_name,a.status,a.summary,a.detail \
             FROM agent_activities a \
             JOIN agent_runs r ON r.id=a.run_id \
             WHERE r.assistant_message_id=?1 \
             ORDER BY a.sequence",
        )
        .await
        .map_err(db)?;
    let mut rows = statement.query([as_i64(id.0)?]).await.map_err(db)?;

    let mut result = Vec::new();

    while let Some(row) = rows.next().await.map_err(db)? {
        result.push(AgentActivity {
            kind: parse_activity_kind(&row.get::<String>(0).map_err(db)?)?,
            call_id: row.get(1).map_err(db)?,
            tool_name: row.get(2).map_err(db)?,
            status: row.get(3).map_err(db)?,
            summary: row.get(4).map_err(db)?,
            detail: row.get(5).map_err(db)?,
        });
    }

    Ok(result)
}

async fn schema_version(connection: &Connection) -> Result<i64> {
    let mut statement = connection
        .prepare("SELECT version FROM _magenta_schema WHERE component='app'")
        .await
        .map_err(db)?;
    let mut rows = statement.query(()).await.map_err(db)?;

    Ok(rows
        .next()
        .await
        .map_err(db)?
        .map(|row| row.get::<i64>(0))
        .transpose()
        .map_err(db)?
        .unwrap_or(0))
}

async fn scalar_i64(
    connection: &Connection,
    sql: &str,
    parameters: impl turso::params::IntoParams,
) -> Result<i64> {
    let mut statement = connection.prepare(sql).await.map_err(db)?;
    let mut rows = statement.query(parameters).await.map_err(db)?;

    let row =
        rows.next().await.map_err(db)?.ok_or_else(|| {
            super::failure(StorageErrorKind::InvalidData, "query returned no value")
        })?;
    row.get(0).map_err(db)
}

fn db(error: turso::Error) -> StorageError {
    StorageError::new(StorageErrorKind::Unavailable, error)
}

fn as_i64(value: u64) -> Result<i64> {
    i64::try_from(value).map_err(super::invalid)
}
fn as_u64(value: i64) -> Result<u64> {
    u64::try_from(value).map_err(super::invalid)
}
const fn mode_name(mode: &ConversationMode) -> &'static str {
    match mode {
        ConversationMode::Chat => "chat",
        ConversationMode::Agent => "agent",
    }
}
fn parse_mode(value: &str) -> Result<ConversationMode> {
    match value {
        "chat" => Ok(ConversationMode::Chat),
        "agent" => Ok(ConversationMode::Agent),
        _ => Err(super::failure(
            StorageErrorKind::InvalidData,
            "unknown conversation mode",
        )),
    }
}
fn parse_role(value: &str) -> Result<MessageRole> {
    match value {
        "user" => Ok(MessageRole::User),
        "assistant" => Ok(MessageRole::Assistant),
        _ => Err(super::failure(
            StorageErrorKind::InvalidData,
            "unknown message role",
        )),
    }
}
const fn status_name(value: MessageStatus) -> &'static str {
    match value {
        MessageStatus::Complete => "complete",
        MessageStatus::Streaming => "streaming",
        MessageStatus::Stopped => "stopped",
        MessageStatus::Failed => "failed",
    }
}
fn parse_status(value: &str) -> Result<MessageStatus> {
    match value {
        "complete" => Ok(MessageStatus::Complete),
        "streaming" => Ok(MessageStatus::Streaming),
        "stopped" => Ok(MessageStatus::Stopped),
        "failed" => Ok(MessageStatus::Failed),
        _ => Err(super::failure(
            StorageErrorKind::InvalidData,
            "unknown message status",
        )),
    }
}
const fn run_status(value: MessageStatus) -> &'static str {
    match value {
        MessageStatus::Complete => "completed",
        MessageStatus::Stopped => "stopped",
        MessageStatus::Failed => "failed",
        MessageStatus::Streaming => "running",
    }
}
const fn activity_kind(value: &AgentActivityKind) -> &'static str {
    match value {
        AgentActivityKind::ToolCall => "tool-call",
        AgentActivityKind::ApprovalRequested => "approval-requested",
        AgentActivityKind::ToolResult => "tool-result",
    }
}
fn parse_activity_kind(value: &str) -> Result<AgentActivityKind> {
    match value {
        "tool-call" => Ok(AgentActivityKind::ToolCall),
        "approval-requested" => Ok(AgentActivityKind::ApprovalRequested),
        "tool-result" => Ok(AgentActivityKind::ToolResult),
        _ => Err(super::failure(
            StorageErrorKind::InvalidData,
            "unknown agent activity kind",
        )),
    }
}
fn decode_optional_path(value: Option<Vec<u8>>) -> Result<Option<PathBuf>> {
    value.map(super::records::decode_path).transpose()
}
fn highlights(value: &str, query: &str) -> Vec<std::ops::Range<usize>> {
    if query.is_empty() {
        return Vec::new();
    }
    value
        .to_lowercase()
        .match_indices(query)
        .map(|(start, text)| start..start + text.len())
        .collect()
}
