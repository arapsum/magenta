use magenta_core::{
    AttachmentDraft, BeginTurn, ConversationId, ConversationMode, ConversationStore, EffortLevel,
    FinishReason, GenerationConfig, GenerationOutcome, MessageFailure, MessageFailureCategory,
    MessageFailureDetail, MessageSequence, MessageStatus, ModelId, Project, ProjectStore,
    ProviderId, StorageErrorKind, Timestamp, TokenUsage,
};
use magenta_storage::SqliteConversationStore;
use std::{
    fs,
    path::{Path, PathBuf},
};

const PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4,
    0x89, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x44, 0x41, 0x54, 0x08, 0x99, 0x63, 0xf8, 0xcf, 0xc0, 0xf0,
    0x1f, 0x00, 0x05, 0x00, 0x01, 0xff, 0x89, 0x99, 0x3d, 0x1d, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45,
    0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];

fn input(id: Option<ConversationId>) -> BeginTurn {
    BeginTurn {
        conversation_id: id,
        title: "Unicode λ and Markdown".into(),
        prompt: "Explain `λ`\n```rust\nfn main() {}\n```".into(),
        attachments: Vec::new(),
        generation: GenerationConfig::new(
            ProviderId::new("openai"),
            ModelId::new("test-model"),
            EffortLevel::Custom {
                value: "budget".into(),
                label: "Thinking budget".into(),
            },
        ),
        mode: ConversationMode::Chat,
        workspace_root: None,
        request_overhead_tokens: 0,
    }
}

fn input_with_attachments(
    id: Option<ConversationId>,
    attachments: Vec<AttachmentDraft>,
) -> BeginTurn {
    BeginTurn {
        attachments,
        ..input(id)
    }
}

fn write_png(directory: &Path, name: &str) -> PathBuf {
    let path = directory.join(name);
    fs::write(&path, PNG).unwrap();
    path
}

#[test]
fn reopen_preserves_messages_configuration_metadata_and_pins() {
    smol::block_on(async {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("history.sqlite3");
        let store = SqliteConversationStore::new(path.clone());
        store.initialize().await.unwrap();
        store.initialize().await.unwrap();
        let pending = store.begin_turn(input(None)).await.unwrap();
        let id = pending.conversation.id;
        let mut assistant = pending.assistant_message;
        assistant.content = "# Answer\n**Unicode** λ\n```rust\nlet x = 1;\n```".into();
        assistant.status = MessageStatus::Complete;
        assistant.generation_outcome = Some(GenerationOutcome::new(
            FinishReason::Other("custom-finish".into()),
            Some(TokenUsage {
                input_tokens: 31,
                output_tokens: 42,
            }),
        ));
        store.finalize(assistant.clone()).await.unwrap();
        store.set_pinned(id, true).await.unwrap();
        drop(store);

        let reopened = SqliteConversationStore::new(path);
        reopened.initialize().await.unwrap();
        let summaries = reopened.summaries().await.unwrap();
        assert_eq!(summaries.len(), 1);
        assert_eq!(
            summaries[0].preview,
            "Explain `λ`\n```rust\nfn main() {}\n```"
        );
        assert!(summaries[0].pinned);
        assert!(summaries[0].updated_at >= summaries[0].created_at);
        let loaded = reopened.load(id).await.unwrap();
        assert_eq!(loaded.conversation, pending.conversation);
        assert_eq!(loaded.page.messages.len(), 2);
        assert_eq!(loaded.page.messages[0].message, pending.user_message);
        assert_eq!(loaded.page.messages[1].message, assistant);
        assert_eq!(loaded.page.messages[0].sequence.0, 0);
        assert_eq!(loaded.page.messages[1].sequence.0, 1);
        assert!(!loaded.page.has_older);
        assert_eq!(loaded.page.messages[1].generation, input(None).generation);

        let continued = reopened.begin_turn(input(Some(id))).await.unwrap();
        assert_eq!(continued.context.len(), 3);
        assert_eq!(continued.context[1], assistant);
        assert!(continued.user_message.id.0 > assistant.id.0);
    });
}

#[test]
fn agent_finalization_marks_the_run_completed() {
    smol::block_on(async {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("history.sqlite3");
        let store = SqliteConversationStore::new(path.clone());
        store.initialize().await.unwrap();

        let mut agent_input = input(None);
        agent_input.mode = ConversationMode::Agent;
        agent_input.workspace_root = Some(directory.path().to_path_buf());
        let pending = store.begin_turn(agent_input).await.unwrap();
        let assistant_id = pending.assistant_message.id.0;
        let mut assistant = pending.assistant_message;
        assistant.status = MessageStatus::Complete;
        assistant.content = "Created the project.".into();

        store.finalize(assistant).await.unwrap();

        let connection = rusqlite::Connection::open(path).unwrap();
        let status: String = connection
            .query_row(
                "SELECT status FROM agent_runs WHERE assistant_message_id = ?1",
                [assistant_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(status, "completed");
    });
}

#[test]
fn rename_persists_without_changing_conversation_recency() {
    smol::block_on(async {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("history.sqlite3");
        let store = SqliteConversationStore::new(path.clone());
        store.initialize().await.unwrap();

        let pending = store.begin_turn(input(None)).await.unwrap();
        let id = pending.conversation.id;
        store.set_pinned(id, true).await.unwrap();
        let before = store.summaries().await.unwrap().remove(0);

        store
            .rename(id, "Renamed conversation".into())
            .await
            .unwrap();
        let renamed = store.summaries().await.unwrap().remove(0);
        assert_eq!(renamed.title, "Renamed conversation");
        assert!(renamed.pinned);
        assert_eq!(renamed.updated_at, before.updated_at);
        assert_eq!(
            store.load(id).await.unwrap().conversation.title,
            renamed.title
        );
        assert_eq!(
            store
                .rename(ConversationId(999), "Missing".into())
                .await
                .unwrap_err()
                .kind,
            StorageErrorKind::NotFound
        );

        drop(store);
        let reopened = SqliteConversationStore::new(path);
        reopened.initialize().await.unwrap();
        assert_eq!(
            reopened.load(id).await.unwrap().conversation.title,
            "Renamed conversation"
        );
    });
}

#[test]
fn full_text_search_tracks_titles_message_updates_renames_and_deletes() {
    smol::block_on(async {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("history.sqlite3");
        let store = SqliteConversationStore::new(path);
        store.initialize().await.unwrap();

        let pending = store.begin_turn(input(None)).await.unwrap();
        let id = pending.conversation.id;
        let title_match = store.search("unic".into(), 10).await.unwrap();
        assert_eq!(title_match.len(), 1);
        assert_eq!(title_match[0].conversation_id, id);
        assert!(title_match[0].message_id.is_none());
        assert_eq!(
            &title_match[0].title[title_match[0].title_highlights[0].clone()],
            "Unicode"
        );

        let body_match = store.search("main".into(), 10).await.unwrap();
        assert_eq!(body_match.len(), 1);
        assert_eq!(body_match[0].message_id, Some(pending.user_message.id));
        assert_eq!(body_match[0].message_sequence.unwrap().0, 0);
        assert!(body_match[0].snippet.contains("main"));
        assert!(!body_match[0].snippet_highlights.is_empty());

        let mut assistant = pending.assistant_message;
        assistant.content = "A distinctly searchable phosphorescent answer".into();
        assistant.status = MessageStatus::Complete;
        store.finalize(assistant.clone()).await.unwrap();
        let finalized = store.search("phosphor".into(), 10).await.unwrap();
        assert_eq!(finalized[0].message_id, Some(assistant.id));

        store.rename(id, "Retitled archive".into()).await.unwrap();
        assert!(store.search("unicode".into(), 10).await.unwrap().is_empty());
        assert_eq!(
            store.search("retit".into(), 10).await.unwrap()[0].conversation_id,
            id
        );

        store.delete(id).await.unwrap();
        assert!(
            store
                .search("phosphor".into(), 10)
                .await
                .unwrap()
                .is_empty()
        );
        assert!(store.search("   ".into(), 10).await.unwrap().is_empty());
    });
}

#[test]
fn version_four_migration_backfills_full_text_indexes() {
    smol::block_on(async {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("history.sqlite3");
        let store = SqliteConversationStore::new(path.clone());
        store.initialize().await.unwrap();
        let pending = store.begin_turn(input(None)).await.unwrap();
        drop(store);

        let connection = rusqlite::Connection::open(&path).unwrap();
        connection
            .execute_batch(
                r"
                    DROP TRIGGER conversation_fts_insert;
                    DROP TRIGGER conversation_fts_delete;
                    DROP TRIGGER conversation_fts_update;
                    DROP TRIGGER message_fts_insert;
                    DROP TRIGGER message_fts_delete;
                    DROP TRIGGER message_fts_update;
                    DROP TABLE conversation_fts;
                    DROP TABLE message_fts;
                    ALTER TABLE messages DROP COLUMN omitted_context_messages;
                    ALTER TABLE messages DROP COLUMN failure;
                    PRAGMA user_version = 4;
                ",
            )
            .unwrap();
        drop(connection);

        let migrated = SqliteConversationStore::new(path);
        migrated.initialize().await.unwrap();
        let matches = migrated.search("main".into(), 10).await.unwrap();
        assert_eq!(matches[0].conversation_id, pending.conversation.id);
        assert_eq!(matches[0].message_id, Some(pending.user_message.id));
    });
}

#[test]
fn delete_removes_conversation_messages_and_attachment_records() {
    smol::block_on(async {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("history.sqlite3");
        let source = write_png(directory.path(), "source.png");
        let store = SqliteConversationStore::new(path.clone());
        store.initialize().await.unwrap();
        let pending = store
            .begin_turn(input_with_attachments(
                None,
                vec![AttachmentDraft {
                    name: "source.png".into(),
                    source_path: source.clone(),
                }],
            ))
            .await
            .unwrap();
        let id = pending.conversation.id;
        let imported = pending.user_message.attachments[0].clone();
        assert!(imported.managed);
        assert!(imported.path.exists());
        assert!(source.exists());

        let connection = rusqlite::Connection::open(&path).unwrap();
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM messages", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            2
        );
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM attachments", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap(),
            1
        );

        store.delete(id).await.unwrap();
        assert!(!imported.path.exists());
        assert!(source.exists());
        assert!(store.summaries().await.unwrap().is_empty());
        assert_eq!(
            store.load(id).await.unwrap_err().kind,
            StorageErrorKind::NotFound
        );
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM messages", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM attachments", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap(),
            0
        );
        assert_eq!(
            store.delete(ConversationId(999)).await.unwrap_err().kind,
            StorageErrorKind::NotFound
        );
    });
}

#[test]
fn pages_are_ordered_without_gaps_and_provider_context_is_independent() {
    smol::block_on(async {
        let directory = tempfile::tempdir().unwrap();
        let store = SqliteConversationStore::new(directory.path().join("history.sqlite3"));
        store.initialize().await.unwrap();
        let mut id = None;
        for index in 0..56 {
            let pending = store.begin_turn(input(id)).await.unwrap();
            id = Some(pending.conversation.id);
            assert_eq!(pending.context.len(), index * 2 + 1);
            let mut assistant = pending.assistant_message;
            assistant.status = MessageStatus::Complete;
            assistant.content = format!("answer {index}");
            store.finalize(assistant).await.unwrap();
        }
        let id = id.unwrap();
        let mut page = store.load(id).await.unwrap().page;
        assert_eq!(page.messages.len(), 50);
        let mut sequences = Vec::new();
        loop {
            assert!(
                page.messages
                    .windows(2)
                    .all(|pair| pair[0].sequence < pair[1].sequence)
            );
            sequences.extend(page.messages.iter().map(|item| item.sequence.0));
            if !page.has_older {
                break;
            }
            page = store.earlier(id, page.older_cursor.unwrap()).await.unwrap();
        }
        sequences.sort_unstable();
        assert_eq!(sequences, (0..112).collect::<Vec<_>>());

        let around = store.load_around(id, MessageSequence(80)).await.unwrap();
        assert!(around.page.has_older);
        assert_eq!(around.page.messages.first().unwrap().sequence.0, 56);
        assert!(
            around
                .page
                .messages
                .iter()
                .any(|message| message.sequence.0 == 80)
        );
    });
}

#[test]
fn interrupted_stream_recovers_once_and_regeneration_keeps_identity() {
    smol::block_on(async {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("history.sqlite3");
        let store = SqliteConversationStore::new(path.clone());
        store.initialize().await.unwrap();
        let pending = store.begin_turn(input(None)).await.unwrap();
        let id = pending.conversation.id;
        store.initialize().await.unwrap();
        assert_eq!(
            store.load(id).await.unwrap().page.messages[1]
                .message
                .status,
            MessageStatus::Streaming
        );
        drop(store);
        let store = SqliteConversationStore::new(path);
        store.initialize().await.unwrap();
        assert_eq!(
            store.load(id).await.unwrap().page.messages[1]
                .message
                .status,
            MessageStatus::Stopped
        );
        let regenerated = store
            .begin_regeneration(id, pending.assistant_message.id, 0)
            .await
            .unwrap();
        assert_eq!(
            regenerated.assistant_message.id,
            pending.assistant_message.id
        );
        assert_eq!(regenerated.context, vec![pending.user_message]);
        let mut assistant = regenerated.assistant_message;
        assistant.status = MessageStatus::Failed;
        assistant.content = "partial response".into();
        store.finalize(assistant.clone()).await.unwrap();
        let loaded = store.load(id).await.unwrap();
        assert_eq!(loaded.page.messages.len(), 2);
        assert_eq!(loaded.page.messages[1].sequence.0, 1);
        assert_eq!(loaded.page.messages[1].message, assistant);
        assert_eq!(
            store.finalize(assistant).await.unwrap_err().kind,
            StorageErrorKind::Conflict
        );
    });
}

#[test]
fn failed_response_persists_only_safe_failure_diagnostics() {
    smol::block_on(async {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("history.sqlite3");
        let store = SqliteConversationStore::new(path.clone());
        store.initialize().await.unwrap();
        let pending = store.begin_turn(input(None)).await.unwrap();
        let mut assistant = pending.assistant_message;
        assistant.status = MessageStatus::Failed;
        assistant.content = "partial output".into();
        assistant.failure = Some(MessageFailure {
            category: MessageFailureCategory::RateLimit,
            reference_code: "MAG-GEN-RATE-LIMIT".into(),
            provider: ProviderId::new("openai"),
            detail: Some(MessageFailureDetail::HttpStatus { status: 429 }),
        });
        store.finalize(assistant.clone()).await.unwrap();

        let reopened = SqliteConversationStore::new(path);
        reopened.initialize().await.unwrap();
        let loaded = reopened.load(pending.conversation.id).await.unwrap();
        assert_eq!(loaded.page.messages[1].message, assistant);
        assert_eq!(loaded.page.messages[1].message.failure, assistant.failure);
        let json = serde_json::to_string(&assistant.failure).unwrap();
        assert!(!json.contains("partial output"));
        assert!(!json.contains("/"));

        let retry = reopened
            .begin_retry(
                pending.conversation.id,
                assistant.id,
                pending.conversation.generation.clone(),
                0,
            )
            .await
            .unwrap();
        assert_ne!(retry.assistant_message.id, assistant.id);
        assert_eq!(retry.context, vec![pending.user_message]);
        let page = reopened.load(pending.conversation.id).await.unwrap().page;
        assert_eq!(page.messages.len(), 3);
        assert_eq!(page.messages[1].message, assistant);
        assert_eq!(page.messages[2].message, retry.assistant_message);
    });
}

#[test]
fn failed_turn_rolls_back_and_newer_schema_is_preserved() {
    smol::block_on(async {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("history.sqlite3");
        let store = SqliteConversationStore::new(path.clone());
        store.initialize().await.unwrap();
        let connection = rusqlite::Connection::open(&path).unwrap();
        connection
            .execute_batch(
                r"
                    CREATE TRIGGER reject_assistant
                    BEFORE INSERT ON messages
                    WHEN NEW.role = 'assistant'
                    BEGIN
                        SELECT RAISE(ABORT, 'test write failure');
                    END;
                ",
            )
            .unwrap();
        assert!(store.begin_turn(input(None)).await.is_err());
        assert!(store.summaries().await.unwrap().is_empty());
        connection
            .execute_batch("DROP TRIGGER reject_assistant; PRAGMA user_version = 99;")
            .unwrap();
        let newer = SqliteConversationStore::new(path);
        assert_eq!(
            newer.initialize().await.unwrap_err().kind,
            StorageErrorKind::UnsupportedVersion
        );
        let version: i64 = connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, 99);
    });
}

#[test]
fn malformed_records_missing_targets_and_busy_turns_return_typed_errors() {
    smol::block_on(async {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("history.sqlite3");
        let store = SqliteConversationStore::new(path.clone());
        assert_eq!(
            store.summaries().await.unwrap_err().kind,
            StorageErrorKind::Unavailable
        );
        store.initialize().await.unwrap();
        assert_eq!(
            store.load(ConversationId(999)).await.unwrap_err().kind,
            StorageErrorKind::NotFound
        );
        let pending = store.begin_turn(input(None)).await.unwrap();
        let result = store.begin_turn(input(Some(pending.conversation.id))).await;
        assert!(matches!(result, Err(error) if error.kind == StorageErrorKind::Conflict));
        let connection = rusqlite::Connection::open(path).unwrap();
        connection
            .execute("UPDATE messages SET generation = 'not JSON'", [])
            .unwrap();
        assert_eq!(
            store.load(pending.conversation.id).await.unwrap_err().kind,
            StorageErrorKind::InvalidData
        );
    });
}

#[test]
fn projects_persist_and_forgetting_one_does_not_delete_conversations() {
    smol::block_on(async {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("history.sqlite3");
        let store = SqliteConversationStore::new(path.clone());
        store.initialize().await.unwrap();
        let pending = store.begin_turn(input(None)).await.unwrap();
        let project_root = directory.path().join("project");
        fs::create_dir(&project_root).unwrap();

        store
            .upsert_project(Project {
                name: "project".into(),
                root: project_root.clone(),
                added_at: Timestamp(1),
                last_opened_at: Timestamp(2),
            })
            .await
            .unwrap();
        assert_eq!(store.projects().await.unwrap()[0].root, project_root);

        store.remove_project(project_root).await.unwrap();
        assert!(store.projects().await.unwrap().is_empty());
        assert_eq!(store.summaries().await.unwrap().len(), 1);
        assert_eq!(
            store
                .load(pending.conversation.id)
                .await
                .unwrap()
                .conversation
                .id,
            pending.conversation.id
        );
    });
}
