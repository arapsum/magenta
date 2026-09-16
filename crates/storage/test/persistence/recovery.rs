use super::*;

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
        assert!(!json.contains('/'));

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
