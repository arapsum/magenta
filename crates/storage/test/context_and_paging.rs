use magenta_core::{
    BeginTurn, ConversationId, ConversationMode, ConversationStore, EffortLevel, GenerationConfig,
    GenerationLimits, MessageStatus, ModelId, ProviderId, StorageErrorKind,
};
use magenta_storage::SqliteConversationStore;

fn input(id: Option<ConversationId>) -> BeginTurn {
    BeginTurn {
        conversation_id: id,
        title: "Context and paging".into(),
        prompt: "latest message".into(),
        attachments: Vec::new(),
        generation: GenerationConfig::new(
            ProviderId::new("openai"),
            ModelId::new("test-model"),
            EffortLevel::Medium,
        ),
        mode: ConversationMode::Chat,
        workspace_root: None,
        request_overhead_tokens: 0,
    }
}

#[test]
fn oversized_newest_turn_rolls_back_the_entire_transaction() {
    smol::block_on(async {
        let directory = tempfile::tempdir().unwrap();
        let store = SqliteConversationStore::new(directory.path().join("history.sqlite3"));
        store.initialize().await.unwrap();
        let mut oversized = input(None);
        oversized.prompt = "x".repeat(300);
        oversized.generation = oversized.generation.with_limits(GenerationLimits {
            context_window_tokens: 300,
            max_output_tokens: 20,
        });

        let error = store.begin_turn(oversized).await.err().unwrap();
        assert_eq!(error.kind, StorageErrorKind::ContextTooLarge);
        assert!(store.summaries().await.unwrap().is_empty());
    });
}

#[test]
fn context_omission_count_is_persisted_on_the_assistant() {
    smol::block_on(async {
        let directory = tempfile::tempdir().unwrap();
        let store = SqliteConversationStore::new(directory.path().join("history.sqlite3"));
        store.initialize().await.unwrap();
        let limits = GenerationLimits {
            context_window_tokens: 350,
            max_output_tokens: 20,
        };
        let mut first = input(None);
        first.prompt = "a".repeat(120);
        first.generation = first.generation.with_limits(limits);
        let first = store.begin_turn(first).await.unwrap();
        let id = first.conversation.id;
        let mut response = first.assistant_message;
        response.content = "b".repeat(120);
        response.status = MessageStatus::Complete;
        store.finalize(response).await.unwrap();

        let mut second = input(Some(id));
        second.prompt = "latest".into();
        second.generation = second.generation.with_limits(limits);
        let second = store.begin_turn(second).await.unwrap();
        assert_eq!(second.context, vec![second.user_message.clone()]);
        assert_eq!(second.context_report.omitted_messages, 2);
        let loaded = store.load(id).await.unwrap();
        assert_eq!(
            loaded
                .page
                .messages
                .last()
                .unwrap()
                .omitted_context_messages,
            2
        );
    });
}

#[test]
fn message_pages_move_in_both_directions() {
    smol::block_on(async {
        let directory = tempfile::tempdir().unwrap();
        let store = SqliteConversationStore::new(directory.path().join("history.sqlite3"));
        store.initialize().await.unwrap();
        let mut id = None;
        for index in 0..55 {
            let pending = store.begin_turn(input(id)).await.unwrap();
            id = Some(pending.conversation.id);
            let mut assistant = pending.assistant_message;
            assistant.content = format!("response {index}");
            assistant.status = MessageStatus::Complete;
            store.finalize(assistant).await.unwrap();
        }
        let id = id.unwrap();
        let latest = store.load(id).await.unwrap().page;
        assert_eq!(latest.messages.len(), 50);
        assert!(latest.has_older);
        assert!(!latest.has_newer);
        let older = store
            .earlier(id, latest.older_cursor.unwrap())
            .await
            .unwrap();
        assert_eq!(older.messages.len(), 50);
        assert!(older.has_older);
        assert!(older.has_newer);
        let oldest = store
            .earlier(id, older.older_cursor.unwrap())
            .await
            .unwrap();
        assert_eq!(oldest.messages.len(), 10);
        assert!(!oldest.has_older);
        assert!(oldest.has_newer);
        let newer = store.later(id, oldest.newer_cursor.unwrap()).await.unwrap();
        assert!(!newer.messages.is_empty());
        assert!(newer.has_older);
        assert!(newer.has_newer);
        let newest = store.later(id, newer.newer_cursor.unwrap()).await.unwrap();
        assert!(newest.has_older);
        assert!(!newest.has_newer);
    });
}
