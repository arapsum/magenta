use magenta_core::{
    Attachment, BeginTurn, ConversationId, ConversationStore, EffortLevel, FinishReason,
    GenerationConfig, GenerationOutcome, MessageStatus, ModelId, ProviderId, StorageErrorKind,
    TokenUsage,
};
use magenta_storage::SqliteConversationStore;

fn input(id: Option<ConversationId>) -> BeginTurn {
    BeginTurn {
        conversation_id: id,
        title: "Unicode λ and Markdown".into(),
        prompt: "Explain `λ`\n```rust\nfn main() {}\n```".into(),
        attachments: vec![Attachment {
            name: "missing.png".into(),
            path: "/nonexistent/reference.png".into(),
        }],
        generation: GenerationConfig::new(
            ProviderId::new("openai"),
            ModelId::new("test-model"),
            EffortLevel::Custom {
                value: "budget".into(),
                label: "Thinking budget".into(),
            },
        ),
    }
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
            .begin_regeneration(id, pending.assistant_message.id)
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
