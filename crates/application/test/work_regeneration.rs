use std::{
    io,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use futures_util::StreamExt as _;
use magenta_application::{RegenerateMessageInput, RunWorkspaceAgent};
use magenta_core::{
    AgentProvider, AgentProviderEvent, AgentProviderStream, AgentRequest, AgentResumeRequest,
    BeginTurn, ConversationMode, ConversationStore, EffortLevel, FinishReason, GenerationConfig,
    GenerationOutcome, MessageStatus, ModelId, ProviderId, WorkspaceAccess, WorkspaceError,
    WorkspaceFuture, WorkspaceMutation, WorkspaceOperation, WorkspacePreview,
};
use magenta_storage::SqliteConversationStore;

#[derive(Default)]
struct RecordingAgent(Mutex<Vec<AgentRequest>>);

impl AgentProvider for RecordingAgent {
    fn start(&self, request: AgentRequest) -> AgentProviderStream {
        self.0.lock().unwrap().push(request);
        Box::pin(futures_util::stream::iter([Ok(
            AgentProviderEvent::Completed(GenerationOutcome::new(FinishReason::Stop, None)),
        )]))
    }

    fn resume(&self, _: AgentResumeRequest) -> AgentProviderStream {
        Box::pin(futures_util::stream::empty())
    }
}

struct UnusedWorkspace;

impl WorkspaceAccess for UnusedWorkspace {
    fn prepare(
        &self,
        _: PathBuf,
        _: WorkspaceOperation,
        _: bool,
    ) -> WorkspaceFuture<WorkspacePreview> {
        Box::pin(async { Err(WorkspaceError::new(io::Error::other("unused"))) })
    }

    fn commit(&self, _: PathBuf, _: WorkspaceMutation) -> WorkspaceFuture<String> {
        Box::pin(async { Err(WorkspaceError::new(io::Error::other("unused"))) })
    }
}

#[test]
fn work_regeneration_uses_agent_tools_and_can_finalize() {
    smol::block_on(async {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("workspace");
        std::fs::create_dir(&root).unwrap();
        let store = Arc::new(SqliteConversationStore::new(
            directory.path().join("history.db"),
        ));
        store.initialize().await.unwrap();
        let pending = store
            .begin_turn(BeginTurn {
                conversation_id: None,
                title: "Work".into(),
                prompt: "Create HelloExpress".into(),
                attachments: Vec::new(),
                generation: GenerationConfig::new(
                    ProviderId::new("test"),
                    ModelId::new("test-model"),
                    EffortLevel::Medium,
                ),
                mode: ConversationMode::Agent,
                workspace_root: Some(root),
                request_overhead_tokens: 0,
            })
            .await
            .unwrap();
        let conversation_id = pending.conversation.id;
        let mut first = pending.assistant_message;
        first.status = MessageStatus::Complete;
        first.content = "Original answer".into();
        store.finalize(first.clone()).await.unwrap();

        let provider = Arc::new(RecordingAgent::default());
        let agent = RunWorkspaceAgent::new(
            provider.clone(),
            store.clone(),
            Arc::new(UnusedWorkspace),
            None,
        );
        let mut replacement = agent
            .regenerate(RegenerateMessageInput {
                conversation_id,
                target_message_id: first.id,
            })
            .await
            .unwrap();
        assert_eq!(replacement.assistant_message.id, first.id);
        while replacement.stream.next().await.is_some() {}
        {
            let requests = provider.0.lock().unwrap();
            assert_eq!(requests.len(), 1);
            assert!(
                requests[0]
                    .tools
                    .iter()
                    .any(|tool| tool.name == "create_directory")
            );
            assert!(
                !requests[0]
                    .tools
                    .iter()
                    .any(|tool| tool.name == "run_command")
            );
            drop(requests);
        }

        replacement.assistant_message.status = MessageStatus::Complete;
        replacement.assistant_message.content = "Replacement answer".into();
        store.finalize(replacement.assistant_message).await.unwrap();
    });
}
