use magenta_core::{
    AgentMemoryStore, AgentSession, AgentSessionState, AgentSessionStore, AssistantTrace,
    AssistantTraceEntry, AssistantTraceKind, AssistantTraceStatus, BeginTurn, CodeChunk, CodeIndex,
    ConversationId, ConversationMode, ConversationStore, EffortLevel, GenerationConfig, MemoryKind,
    MemoryState, MessageStatus, ModelId, NewAgentMemory, Project, ProjectStore, ProviderId,
    Timestamp,
};
use magenta_storage::{TursoAgentDatabase, TursoAppStore};
use turso::{Builder, params};

fn generation() -> GenerationConfig {
    GenerationConfig::new(
        ProviderId::new("openai"),
        ModelId::new("test-model"),
        EffortLevel::Medium,
    )
}

#[test]
fn local_turso_app_store_persists_conversations_and_projects() {
    smol::block_on(async {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("magenta.db");

        let store = TursoAppStore::new(path.clone());
        store.initialize().await.unwrap();

        let pending = store
            .begin_turn(BeginTurn {
                conversation_id: None,
                title: "Turso conversation".into(),
                prompt: "remember the architecture".into(),
                attachments: Vec::new(),
                generation: generation(),
                mode: ConversationMode::Chat,
                workspace_root: None,
                request_overhead_tokens: 0,
            })
            .await
            .unwrap();

        let id = pending.conversation.id;
        let mut assistant = pending.assistant_message;
        assistant.content = "Done".into();
        assistant.status = MessageStatus::Complete;
        store.finalize(assistant).await.unwrap();

        let root = directory.path().join("project");
        store
            .upsert_project(Project {
                name: "Project".into(),
                root: root.clone(),
                added_at: Timestamp(1),
                last_opened_at: Timestamp(2),
            })
            .await
            .unwrap();

        drop(store);

        let reopened = TursoAppStore::new(path);
        reopened.initialize().await.unwrap();

        assert_eq!(reopened.load(id).await.unwrap().page.messages.len(), 2);
        assert_eq!(
            reopened
                .search("architecture".into(), 10)
                .await
                .unwrap()
                .len(),
            1
        );
        assert_eq!(reopened.projects().await.unwrap()[0].root, root);
    });
}

#[test]
fn local_turso_agent_regeneration_can_finalize_a_replaced_answer() {
    smol::block_on(async {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("workspace");
        std::fs::create_dir(&root).unwrap();
        let store = TursoAppStore::new(directory.path().join("magenta.db"));
        store.initialize().await.unwrap();
        let pending = store
            .begin_turn(BeginTurn {
                conversation_id: None,
                title: "Work".into(),
                prompt: "Create a folder".into(),
                attachments: Vec::new(),
                generation: generation(),
                mode: ConversationMode::Agent,
                workspace_root: Some(root),
                request_overhead_tokens: 0,
            })
            .await
            .unwrap();
        let mut first = pending.assistant_message;
        first.content = "First answer".into();
        first.status = MessageStatus::Complete;
        store.finalize(first.clone()).await.unwrap();

        let regenerated = store
            .begin_regeneration(pending.conversation.id, first.id, 0)
            .await
            .unwrap();
        let mut replacement = regenerated.assistant_message;
        replacement.content = "Replacement answer".into();
        replacement.status = MessageStatus::Complete;
        store.finalize(replacement).await.unwrap();
    });
}

#[test]
fn local_turso_finalize_waits_for_a_concurrent_trace_writer() {
    smol::block_on(async {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("magenta.db");
        let store = TursoAppStore::new(path.clone());
        store.initialize().await.unwrap();
        let pending = store
            .begin_turn(BeginTurn {
                conversation_id: None,
                title: "Concurrent writes".into(),
                prompt: "Work".into(),
                attachments: Vec::new(),
                generation: generation(),
                mode: ConversationMode::Agent,
                workspace_root: Some(directory.path().to_path_buf()),
                request_overhead_tokens: 0,
            })
            .await
            .unwrap();
        let mut assistant = pending.assistant_message;
        assistant.content = "Finished".into();
        assistant.status = MessageStatus::Complete;

        let database = Builder::new_local(&path.to_string_lossy())
            .build()
            .await
            .unwrap();
        let mut connection = database.connect().unwrap();
        let transaction = connection
            .transaction_with_behavior(turso::transaction::TransactionBehavior::Immediate)
            .await
            .unwrap();
        transaction
            .execute(
                "UPDATE messages SET thinking_duration_ms=1 WHERE id=?1",
                [i64::try_from(assistant.id.0).unwrap()],
            )
            .await
            .unwrap();

        let finalize = smol::spawn(async move { store.finalize(assistant).await });
        smol::Timer::after(std::time::Duration::from_millis(150)).await;
        transaction.commit().await.unwrap();
        finalize.await.unwrap();
    });
}

#[test]
fn project_database_separates_memory_code_and_sessions_by_root() {
    smol::block_on(async {
        let directory = tempfile::tempdir().unwrap();
        let database = TursoAgentDatabase::new(directory.path().join("projects"));
        let first = directory.path().join("first");
        let second = directory.path().join("second");

        let memory = database
            .remember(
                first.clone(),
                NewAgentMemory {
                    kind: MemoryKind::Decision,
                    state: MemoryState::Active,
                    content: "Use hexagonal architecture for storage".into(),
                    source_conversation_id: None,
                    confidence: 0.9,
                    embedding: Some(vec![1.0, 0.0]),
                },
            )
            .await
            .unwrap();

        assert!(memory.id > 0);
        assert_eq!(
            database
                .recall(
                    first.clone(),
                    "hexagonal storage".into(),
                    Some(vec![1.0, 0.0]),
                    8
                )
                .await
                .unwrap()
                .len(),
            1
        );

        assert!(
            database
                .recall(second, "hexagonal storage".into(), Some(vec![1.0, 0.0]), 8)
                .await
                .unwrap()
                .is_empty()
        );

        database
            .replace_file(
                first.clone(),
                "src/lib.rs".into(),
                "hash".into(),
                vec![CodeChunk {
                    path: "src/lib.rs".into(),
                    language: "rust".into(),
                    symbol: Some("adapter".into()),
                    start_line: 1,
                    end_line: 3,
                    content: "pub struct TursoAdapter;".into(),
                    content_hash: "hash".into(),
                    embedding: None,
                    session_id: None,
                }],
            )
            .await
            .unwrap();
        assert_eq!(
            database
                .search_code(first.clone(), "TursoAdapter".into(), None, None, 8)
                .await
                .unwrap()
                .len(),
            1
        );

        database
            .create_session(
                first.clone(),
                AgentSession {
                    id: "run-1".into(),
                    conversation_id: magenta_core::ConversationId(7),
                    agentfs_path: directory.path().join("run-1.db"),
                    state: AgentSessionState::Running,
                    created_at: Timestamp(1),
                    updated_at: Timestamp(1),
                },
            )
            .await
            .unwrap();
        database
            .set_session_state(
                first.clone(),
                "run-1".into(),
                AgentSessionState::AwaitingReview,
            )
            .await
            .unwrap();
        assert_eq!(
            database
                .session(first, "run-1".into())
                .await
                .unwrap()
                .unwrap()
                .state,
            AgentSessionState::AwaitingReview
        );
    });
}

#[test]
fn local_turso_app_store_persists_ordered_assistant_traces_and_duration() {
    smol::block_on(async {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("trace.db");
        let store = TursoAppStore::new(path.clone());
        store.initialize().await.unwrap();

        let pending = store
            .begin_turn(BeginTurn {
                conversation_id: None,
                title: "Trace conversation".into(),
                prompt: "show the work".into(),
                attachments: Vec::new(),
                generation: generation(),
                mode: ConversationMode::Chat,
                workspace_root: None,
                request_overhead_tokens: 0,
            })
            .await
            .unwrap();
        let conversation_id = pending.conversation.id;
        let assistant_id = pending.assistant_message.id;
        let mut assistant = pending.assistant_message;
        assistant.content = "Finished".into();
        assistant.status = MessageStatus::Complete;
        assistant.assistant_trace = AssistantTrace {
            entries: vec![
                AssistantTraceEntry {
                    key: "reasoning:rs_1:0".into(),
                    sequence: 0,
                    kind: AssistantTraceKind::ReasoningSummary,
                    status: AssistantTraceStatus::Completed,
                    title: "Thinking".into(),
                    tool_name: None,
                    input: String::new(),
                    output: "Checked the request.".into(),
                    started_at: Some(Timestamp(10)),
                    finished_at: Some(Timestamp(20)),
                },
                AssistantTraceEntry {
                    key: "tool:call_1".into(),
                    sequence: 1,
                    kind: AssistantTraceKind::Tool,
                    status: AssistantTraceStatus::Completed,
                    title: "List files".into(),
                    tool_name: Some("list_files".into()),
                    input: "{\"path\":\".\"}".into(),
                    output: "README.md".into(),
                    started_at: Some(Timestamp(21)),
                    finished_at: Some(Timestamp(30)),
                },
            ],
            thinking_duration_ms: Some(30),
        };
        store.finalize(assistant).await.unwrap();
        drop(store);

        let reopened = TursoAppStore::new(path);
        reopened.initialize().await.unwrap();
        let page = reopened.load(conversation_id).await.unwrap();
        let saved = page
            .page
            .messages
            .iter()
            .find(|message| message.message.id == assistant_id)
            .expect("assistant message should reload");
        assert_eq!(saved.message.assistant_trace.thinking_duration_ms, Some(30));
        assert_eq!(
            saved
                .message
                .assistant_trace
                .entries
                .iter()
                .map(|entry| entry.key.as_str())
                .collect::<Vec<_>>(),
            vec!["reasoning:rs_1:0", "tool:call_1"]
        );

        reopened.delete(conversation_id).await.unwrap();
        assert!(reopened.load(conversation_id).await.is_err());
    });
}

#[test]
#[allow(clippy::too_many_lines)]
fn local_turso_v1_migration_backfills_one_trace_entry_per_tool_call() {
    smol::block_on(async {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("legacy.db");
        let database = Builder::new_local(&path.to_string_lossy())
            .build()
            .await
            .unwrap();
        let connection = database.connect().unwrap();
        connection
            .execute_batch(
                r"
                CREATE TABLE _magenta_schema(component TEXT PRIMARY KEY, version INTEGER NOT NULL);
                INSERT INTO _magenta_schema(component, version) VALUES ('app', 1);
                CREATE TABLE conversations (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    title TEXT NOT NULL,
                    generation TEXT NOT NULL,
                    mode TEXT NOT NULL,
                    workspace_root BLOB,
                    pinned INTEGER NOT NULL DEFAULT 0,
                    created_at INTEGER NOT NULL,
                    updated_at INTEGER NOT NULL
                );
                CREATE TABLE messages (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    conversation_id INTEGER NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
                    sequence INTEGER NOT NULL,
                    role TEXT NOT NULL,
                    content TEXT NOT NULL,
                    status TEXT NOT NULL,
                    generation TEXT NOT NULL,
                    outcome TEXT,
                    failure TEXT,
                    omitted_context_messages INTEGER NOT NULL DEFAULT 0,
                    created_at INTEGER NOT NULL,
                    UNIQUE(conversation_id, sequence)
                );
                CREATE UNIQUE INDEX one_stream_per_conversation
                    ON messages(conversation_id) WHERE status = 'streaming';
                CREATE INDEX message_conversation_order
                    ON messages(conversation_id, sequence);
                CREATE TABLE projects (
                    root BLOB PRIMARY KEY,
                    name TEXT NOT NULL,
                    added_at INTEGER NOT NULL,
                    last_opened_at INTEGER NOT NULL
                );
                CREATE TABLE agent_runs (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    conversation_id INTEGER NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
                    assistant_message_id INTEGER NOT NULL UNIQUE REFERENCES messages(id) ON DELETE CASCADE,
                    status TEXT NOT NULL,
                    started_at INTEGER NOT NULL,
                    finished_at INTEGER
                );
                CREATE TABLE agent_activities (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    run_id INTEGER NOT NULL REFERENCES agent_runs(id) ON DELETE CASCADE,
                    sequence INTEGER NOT NULL,
                    kind TEXT NOT NULL,
                    call_id TEXT NOT NULL,
                    tool_name TEXT NOT NULL,
                    status TEXT NOT NULL,
                    summary TEXT NOT NULL,
                    detail TEXT NOT NULL,
                    created_at INTEGER NOT NULL,
                    UNIQUE(run_id, sequence)
                );
                CREATE INDEX agent_activity_order ON agent_activities(run_id, sequence);
                CREATE TABLE attachments (
                    message_id INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
                    position INTEGER NOT NULL,
                    name TEXT NOT NULL,
                    source_path BLOB NOT NULL,
                    mime_type TEXT NOT NULL,
                    byte_size INTEGER NOT NULL,
                    managed INTEGER NOT NULL,
                    PRIMARY KEY(message_id, position)
                );
                ",
            )
            .await
            .unwrap();

        let generation = serde_json::to_string(&generation()).unwrap();
        connection
            .execute(
                "INSERT INTO conversations(id,title,generation,mode,created_at,updated_at) \
                 VALUES (1,'Legacy',?1,'agent',1,2)",
                params![generation.as_str()],
            )
            .await
            .unwrap();
        connection
            .execute(
                "INSERT INTO messages(id,conversation_id,sequence,role,content,status,generation,created_at) \
                 VALUES (1,1,0,'user','Inspect this','complete',?1,1), \
                        (2,1,1,'assistant','','streaming',?1,2)",
                params![generation.as_str()],
            )
            .await
            .unwrap();
        connection
            .execute(
                "INSERT INTO agent_runs(id,conversation_id,assistant_message_id,status,started_at) \
                 VALUES (1,1,2,'running',2)",
                (),
            )
            .await
            .unwrap();
        connection
            .execute(
                "INSERT INTO agent_activities( \
                 run_id,sequence,kind,call_id,tool_name,status,summary,detail,created_at \
                 ) VALUES (1,0,'tool-call','call_1','read_file','requested','Read file','{\"path\":\"README.md\"}',3), \
                          (1,1,'approval-requested','call_1','read_file','awaiting-approval','Approval','README.md',4), \
                          (1,2,'tool-result','call_1','read_file','completed','Read file','contents',5)",
                (),
            )
            .await
            .unwrap();
        drop(connection);
        drop(database);

        let store = TursoAppStore::new(path);
        store.initialize().await.unwrap();
        let page = store.load(ConversationId(1)).await.unwrap();
        let assistant = page
            .page
            .messages
            .iter()
            .find(|message| message.message.id.0 == 2)
            .expect("legacy assistant should reload");
        assert_eq!(assistant.message.assistant_trace.entries.len(), 1);
        let entry = &assistant.message.assistant_trace.entries[0];
        assert_eq!(entry.key, "tool:call_1");
        assert_eq!(entry.sequence, 0);
        assert_eq!(entry.output, "contents");
        assert_eq!(entry.status, AssistantTraceStatus::Completed);

        let reopened = TursoAppStore::new(directory.path().join("legacy.db"));
        reopened.initialize().await.unwrap();
        let reloaded = reopened.load(ConversationId(1)).await.unwrap();
        assert_eq!(
            reloaded
                .page
                .messages
                .iter()
                .find(|message| message.message.id.0 == 2)
                .unwrap()
                .message
                .assistant_trace
                .entries
                .len(),
            1
        );
    });
}

#[test]
fn local_turso_app_store_repairs_orphaned_message_autoindex() {
    smol::block_on(async {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("orphaned.db");
        let database = Builder::new_local(&path.to_string_lossy())
            .build()
            .await
            .unwrap();
        let connection = database.connect().unwrap();
        connection
            .execute_batch(
                r"
                CREATE TABLE _magenta_schema(component TEXT PRIMARY KEY, version INTEGER NOT NULL);
                INSERT INTO _magenta_schema(component, version) VALUES ('app', 2);
                CREATE TABLE conversations (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    title TEXT NOT NULL,
                    generation TEXT NOT NULL,
                    mode TEXT NOT NULL,
                    workspace_root BLOB,
                    pinned INTEGER NOT NULL DEFAULT 0,
                    created_at INTEGER NOT NULL,
                    updated_at INTEGER NOT NULL
                );
                CREATE TABLE messages (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    conversation_id INTEGER NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
                    sequence INTEGER NOT NULL,
                    role TEXT NOT NULL,
                    content TEXT NOT NULL,
                    status TEXT NOT NULL,
                    generation TEXT NOT NULL,
                    outcome TEXT,
                    failure TEXT,
                    omitted_context_messages INTEGER NOT NULL DEFAULT 0,
                    created_at INTEGER NOT NULL,
                    UNIQUE(conversation_id, sequence)
                );
                ",
            )
            .await
            .unwrap();
        let generation = serde_json::to_string(&generation()).unwrap();
        connection
            .execute(
                "INSERT INTO conversations(id,title,generation,mode,created_at,updated_at) \
                 VALUES (1,'Repair',?1,'chat',1,2)",
                params![generation.as_str()],
            )
            .await
            .unwrap();
        connection
            .execute(
                "INSERT INTO messages(id,conversation_id,sequence,role,content,status,generation,created_at) \
                 VALUES (1,1,0,'assistant','Recovered','complete',?1,2)",
                params![generation.as_str()],
            )
            .await
            .unwrap();
        connection
            .execute(
                "ALTER TABLE messages ADD COLUMN thinking_duration_ms INTEGER",
                (),
            )
            .await
            .unwrap();
        connection
            .execute(
                "UPDATE conversations SET generation=?1",
                params![generation.as_str()],
            )
            .await
            .unwrap();
        connection
            .execute(
                "UPDATE messages SET generation=?1",
                params![generation.as_str()],
            )
            .await
            .unwrap();
        drop(connection);
        drop(database);

        let store = TursoAppStore::new(path.clone());
        store.initialize().await.unwrap();
        let page = store.load(ConversationId(1)).await.unwrap();
        assert_eq!(page.page.messages[0].message.content, "Recovered");

        let reopened = TursoAppStore::new(path);
        reopened.initialize().await.unwrap();
        assert_eq!(
            reopened
                .load(ConversationId(1))
                .await
                .unwrap()
                .page
                .messages[0]
                .message
                .content,
            "Recovered"
        );
    });
}
