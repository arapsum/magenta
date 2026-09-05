use magenta_application::{
    RegenerateMessage, RegenerateMessageInput, SendMessage, SendMessageInput, SendTarget,
};
use magenta_core::{
    ChatProvider, ConversationStore, EffortLevel, GenerationConfig, GenerationRequest,
    GenerationStream, MessageStatus, ModelId, ProviderId,
};
use magenta_storage::SqliteConversationStore;
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct RecordingProvider(Mutex<Vec<GenerationRequest>>);

impl ChatProvider for RecordingProvider {
    fn stream(&self, request: GenerationRequest) -> GenerationStream {
        self.0.lock().unwrap().push(request);
        Box::pin(futures_util::stream::empty())
    }
}

fn input(target: SendTarget) -> SendMessageInput {
    SendMessageInput {
        target,
        prompt: "  A durable conversation  ".into(),
        attachments: Vec::new(),
        generation: GenerationConfig::new(
            ProviderId::new("test"),
            ModelId::new("model"),
            EffortLevel::High,
        ),
    }
}

#[test]
fn storage_failure_and_empty_prompt_never_invoke_the_provider() {
    smol::block_on(async {
        let directory = tempfile::tempdir().unwrap();
        let store = Arc::new(SqliteConversationStore::new(
            directory.path().join("history.sqlite3"),
        ));
        let provider = Arc::new(RecordingProvider::default());
        let workflow = SendMessage::new(provider.clone(), store);
        assert!(workflow.execute(input(SendTarget::New)).await.is_err());
        let mut empty = input(SendTarget::New);
        empty.prompt = " \n ".into();
        assert!(workflow.execute(empty).await.is_err());
        assert!(provider.0.lock().unwrap().is_empty());
    });
}

#[test]
fn reopened_send_and_regeneration_use_committed_history() {
    smol::block_on(async {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("history.sqlite3");
        let store = Arc::new(SqliteConversationStore::new(path.clone()));
        store.initialize().await.unwrap();
        let provider = Arc::new(RecordingProvider::default());
        let workflow = SendMessage::new(provider.clone(), store.clone());
        let pending = workflow.execute(input(SendTarget::New)).await.unwrap();
        let id = pending.conversation.id;
        assert_eq!(pending.conversation.title, "A durable conversation");
        assert_eq!(store.load(id).await.unwrap().page.messages.len(), 2);
        let mut assistant = pending.assistant_message;
        assistant.status = MessageStatus::Complete;
        assistant.content = "Remember this answer".into();
        store.finalize(assistant.clone()).await.unwrap();
        drop(workflow);
        drop(store);

        let store = Arc::new(SqliteConversationStore::new(path));
        store.initialize().await.unwrap();
        let workflow = SendMessage::new(provider.clone(), store.clone());
        let next = workflow
            .execute(input(SendTarget::Existing(id)))
            .await
            .unwrap();
        assert_eq!(provider.0.lock().unwrap()[1].messages[1], assistant);
        let mut stopped = next.assistant_message;
        stopped.status = MessageStatus::Stopped;
        store.finalize(stopped.clone()).await.unwrap();
        let regeneration = RegenerateMessage::new(provider.clone(), store.clone());
        let replacement = regeneration
            .execute(RegenerateMessageInput {
                conversation_id: id,
                target_message_id: assistant.id,
            })
            .await
            .unwrap();
        assert_eq!(replacement.assistant_message.id, assistant.id);
        assert_eq!(provider.0.lock().unwrap()[2].messages.len(), 1);
        assert_eq!(
            store.load(id).await.unwrap().page.messages[3].message,
            stopped
        );
    });
}
