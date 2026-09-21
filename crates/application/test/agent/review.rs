use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

use magenta_core::{
    AgentDataFuture, AgentSession, AgentSessionState, AgentSessionStore, MessageId,
    WorkspaceFuture, WorkspaceMutation, WorkspaceOperation, WorkspacePreview,
};

use super::*;

struct ReviewWorkspace {
    has_changes: bool,
}

impl WorkspaceAccess for ReviewWorkspace {
    fn prepare(
        &self,
        _: PathBuf,
        _: WorkspaceOperation,
        _: bool,
    ) -> WorkspaceFuture<WorkspacePreview> {
        unreachable!("review lifecycle tests do not execute workspace tools")
    }

    fn commit(&self, _: PathBuf, _: WorkspaceMutation) -> WorkspaceFuture<String> {
        unreachable!("review lifecycle tests do not commit workspace tools")
    }
}

impl WorkspaceSessionAccess for ReviewWorkspace {
    fn ensure_session_available(&self, _: PathBuf) -> WorkspaceFuture<()> {
        Box::pin(async { Ok(()) })
    }

    fn start_session(&self, _: PathBuf, _: String) -> WorkspaceFuture<PathBuf> {
        Box::pin(async { Ok(PathBuf::from("review.db")) })
    }

    fn session_has_changes(&self, _: PathBuf, _: String) -> WorkspaceFuture<bool> {
        let has_changes = self.has_changes;
        Box::pin(async move { Ok(has_changes) })
    }

    fn apply_session(&self, _: PathBuf, _: String) -> WorkspaceFuture<Vec<String>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    fn discard_session(&self, _: PathBuf, _: String) -> WorkspaceFuture<()> {
        Box::pin(async { Ok(()) })
    }
}

#[derive(Default)]
struct ReviewStore {
    states: Mutex<Vec<AgentSessionState>>,
}

impl AgentSessionStore for ReviewStore {
    fn create_session(&self, _: PathBuf, _: AgentSession) -> AgentDataFuture<()> {
        Box::pin(async { Ok(()) })
    }

    fn set_session_state(
        &self,
        _: PathBuf,
        _: String,
        state: AgentSessionState,
    ) -> AgentDataFuture<()> {
        self.states.lock().unwrap().push(state);
        Box::pin(async { Ok(()) })
    }

    fn session(&self, _: PathBuf, _: String) -> AgentDataFuture<Option<AgentSession>> {
        Box::pin(async { Ok(None) })
    }
}

fn controller(has_changes: bool) -> (AgentApprovalController, Arc<ReviewStore>) {
    let store = Arc::new(ReviewStore::default());
    let review = AgentReviewHandle {
        workspace: Arc::new(ReviewWorkspace { has_changes }),
        store: store.clone(),
        root: PathBuf::from("workspace"),
        session_id: "session".to_owned(),
        assistant_message_id: MessageId::new(7),
        pending: Arc::new(AtomicBool::new(false)),
    };
    let (sender, _receiver) = async_channel::unbounded();

    (
        AgentApprovalController {
            sender,
            review: Some(review),
        },
        store,
    )
}

#[test]
fn review_is_not_pending_until_staged_changes_are_confirmed() {
    smol::block_on(async {
        let (controller, store) = controller(true);

        assert!(!controller.has_pending_workspace_review());
        controller.finish_workspace_review().await.unwrap();
        assert!(controller.has_pending_workspace_review());
        assert_eq!(
            *store.states.lock().unwrap(),
            vec![AgentSessionState::AwaitingReview]
        );
    });
}

#[test]
fn an_empty_review_session_is_discarded_without_prompting() {
    smol::block_on(async {
        let (controller, store) = controller(false);

        controller.finish_workspace_review().await.unwrap();
        assert!(!controller.has_pending_workspace_review());
        assert_eq!(
            *store.states.lock().unwrap(),
            vec![AgentSessionState::Discarded]
        );
    });
}
