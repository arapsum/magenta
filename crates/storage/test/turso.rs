use magenta_core::{
    AgentMemoryStore, AgentSession, AgentSessionState, AgentSessionStore, BeginTurn, CodeChunk,
    CodeIndex, ConversationMode, ConversationStore, EffortLevel, GenerationConfig, MemoryKind,
    MemoryState, MessageStatus, ModelId, NewAgentMemory, Project, ProjectStore, ProviderId,
    Timestamp,
};
use magenta_storage::{TursoAgentDatabase, TursoAppStore};

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
