mod run;
mod stream;
mod tool;

#[cfg(test)]
#[path = "../../test/agent/review.rs"]
mod review_tests;

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use async_channel::Sender;
use magenta_core::{
    AgentApprovalDecision, AgentContentCache, AgentMemoryStore, AgentProvider, AgentRequest,
    AgentRunStream, AgentSession, AgentSessionState, AgentSessionStore, AgentToolDefinition,
    AgentToolPolicy, BeginTurn, CodeIndex, CodeIndexMaintainer, CommandCatalog, Conversation,
    ConversationId, ConversationMode, ConversationStore, EmbeddingProvider, GenerationConfig,
    MemoryKind, MemoryState, Message, NewAgentMemory, PreparedTurn, ProviderId, RepositoryAccess,
    RetrievedContextBlock, RetrievedContextKind, WorkspaceAccess, WorkspaceCommandRunner,
    WorkspaceSessionAccess, estimate_agent_overhead,
};

use super::trace::AssistantTraceRecorder;
use crate::{RetryMessageError, SendMessageError, conversation_title};

const MAX_AGENT_ROUNDS: usize = 64;
const MAX_AGENT_TOOL_CALLS: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentSendTarget {
    New,
    Existing(ConversationId),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunWorkspaceAgentInput {
    pub target: AgentSendTarget,
    pub prompt: String,
    pub command_id: Option<magenta_core::CommandId>,
    pub generation: GenerationConfig,
    pub workspace_root: std::path::PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RetryWorkspaceAgentInput {
    pub conversation_id: ConversationId,
    pub target_message_id: magenta_core::MessageId,
    pub generation: GenerationConfig,
}

pub struct PendingAgentGeneration {
    pub conversation: Conversation,
    pub user_message: Message,
    pub assistant_message: Message,
    pub stream: AgentRunStream,
    pub controller: AgentApprovalController,
    pub user_sequence: magenta_core::MessageSequence,
    pub assistant_sequence: magenta_core::MessageSequence,
    pub context_report: magenta_core::ContextBudgetReport,
}

#[derive(Clone)]
pub struct AgentStreamContext {
    pub provider: Arc<dyn AgentProvider>,
    pub workspace: Arc<dyn WorkspaceAccess>,
    pub command_runner: Option<Arc<dyn WorkspaceCommandRunner>>,
    pub repository: Option<Arc<dyn RepositoryAccess>>,
    pub tool_policy: AgentToolPolicy,
    pub root: std::path::PathBuf,
    pub conversation: Conversation,
    pub trace: AssistantTraceRecorder,
    pub review: Option<AgentReviewHandle>,
    context_services: Option<AgentContextServices>,
}

#[derive(Clone)]
pub struct AgentApprovalController {
    sender: Sender<ApprovalResponse>,
    review: Option<AgentReviewHandle>,
}

impl AgentApprovalController {
    pub fn decide(&self, request_id: impl Into<String>, decision: AgentApprovalDecision) -> bool {
        self.sender
            .try_send(ApprovalResponse {
                request_id: request_id.into(),
                decision,
            })
            .is_ok()
    }

    #[must_use]
    pub fn has_pending_workspace_review(&self) -> bool {
        self.review
            .as_ref()
            .is_some_and(AgentReviewHandle::is_pending)
    }

    #[must_use]
    pub fn is_workspace_review_for(&self, message_id: magenta_core::MessageId) -> bool {
        self.review
            .as_ref()
            .is_some_and(|review| review.is_pending() && review.assistant_message_id == message_id)
    }

    /// Closes an empty session after a failed or stopped Work run, or retains
    /// staged changes for review.
    ///
    /// # Errors
    ///
    /// Returns an error if the `AgentFS` session cannot be inspected or closed.
    pub async fn finish_workspace_review(&self) -> Result<(), magenta_core::WorkspaceError> {
        if let Some(review) = &self.review {
            review.finish().await?;
        }
        Ok(())
    }

    /// Applies all staged changes in the pending workspace review.
    ///
    /// # Errors
    ///
    /// Returns an error when there is no pending review or the workspace
    /// cannot apply the review session.
    pub async fn apply_workspace_changes(
        &self,
    ) -> Result<Vec<String>, magenta_core::WorkspaceError> {
        let review = self.review.as_ref().ok_or_else(|| {
            magenta_core::WorkspaceError::new(std::io::Error::other("no pending workspace review"))
        })?;

        let changes = review
            .workspace
            .apply_session(review.root.clone(), review.session_id.clone())
            .await?;
        review.pending.store(false, Ordering::Release);

        let _ = review
            .store
            .set_session_state(
                review.root.clone(),
                review.session_id.clone(),
                AgentSessionState::Applied,
            )
            .await;
        Ok(changes)
    }

    /// Discards all staged changes in the pending workspace review.
    ///
    /// # Errors
    ///
    /// Returns an error when there is no pending review or the workspace
    /// cannot discard the review session.
    pub async fn discard_workspace_changes(&self) -> Result<(), magenta_core::WorkspaceError> {
        let review = self.review.as_ref().ok_or_else(|| {
            magenta_core::WorkspaceError::new(std::io::Error::other("no pending workspace review"))
        })?;

        review
            .workspace
            .discard_session(review.root.clone(), review.session_id.clone())
            .await?;
        review.pending.store(false, Ordering::Release);

        let _ = review
            .store
            .set_session_state(
                review.root.clone(),
                review.session_id.clone(),
                AgentSessionState::Discarded,
            )
            .await;
        Ok(())
    }
}

#[derive(Clone)]
pub struct AgentReviewHandle {
    workspace: Arc<dyn WorkspaceSessionAccess>,
    store: Arc<dyn AgentSessionStore>,
    root: std::path::PathBuf,
    session_id: String,
    assistant_message_id: magenta_core::MessageId,
    pending: Arc<AtomicBool>,
}

impl AgentReviewHandle {
    fn is_pending(&self) -> bool {
        self.pending.load(Ordering::Acquire)
    }

    pub(crate) async fn finish(&self) -> Result<(), magenta_core::WorkspaceError> {
        if !self
            .workspace
            .session_has_changes(self.root.clone(), self.session_id.clone())
            .await?
        {
            self.workspace
                .discard_session(self.root.clone(), self.session_id.clone())
                .await?;
            self.pending.store(false, Ordering::Release);
            let _ = self
                .store
                .set_session_state(
                    self.root.clone(),
                    self.session_id.clone(),
                    AgentSessionState::Discarded,
                )
                .await;
            return Ok(());
        }
        let _ = self
            .store
            .set_session_state(
                self.root.clone(),
                self.session_id.clone(),
                AgentSessionState::AwaitingReview,
            )
            .await;
        self.pending.store(true, Ordering::Release);
        Ok(())
    }
}

pub struct ApprovalResponse {
    pub request_id: String,
    pub decision: AgentApprovalDecision,
}

#[derive(Clone)]
pub struct RunWorkspaceAgent {
    provider: Arc<dyn AgentProvider>,
    store: Arc<dyn ConversationStore>,
    workspace: Arc<dyn WorkspaceAccess>,
    command_runner: Option<Arc<dyn WorkspaceCommandRunner>>,
    repository: Option<Arc<dyn RepositoryAccess>>,
    command_catalog: Option<Arc<dyn CommandCatalog>>,
    context_services: Option<AgentContextServices>,
    code_indexer: Option<Arc<dyn CodeIndexMaintainer>>,
    workspace_sessions: Option<(Arc<dyn WorkspaceSessionAccess>, Arc<dyn AgentSessionStore>)>,
}

#[derive(Clone)]
struct AgentContextServices {
    memories: Arc<dyn AgentMemoryStore>,
    code: Arc<dyn CodeIndex>,
    embeddings: Arc<dyn EmbeddingProvider>,
    cache: Arc<dyn AgentContentCache>,
}

fn instructions_with_context(
    mut instructions: String,
    context: &[RetrievedContextBlock],
) -> String {
    if context.is_empty() {
        return instructions;
    }
    instructions.push_str(
        "\n\nProject context (retrieved, potentially stale; verify against workspace):\n",
    );
    for block in context {
        use std::fmt::Write as _;
        let _ = writeln!(instructions, "\n[{}]\n{}", block.source, block.content);
    }
    instructions
}

pub fn agent_error(provider: &ProviderId, message: &str) -> magenta_core::ProviderError {
    magenta_core::ProviderError::with_kind(
        provider.clone(),
        magenta_core::ProviderErrorKind::Other,
        std::io::Error::other(message.to_owned()),
    )
}

fn agent_instructions(root: &std::path::Path) -> String {
    format!(
        concat!(
            "You are Magenta's constrained workspace execution agent operating in {}. ",
            "You have actual access to this workspace through the supplied tools. ",
            "You MUST use those tools to inspect, create, or edit files when the user asks ",
            "for a workspace change. Use run_command after edits when a relevant ",
            "non-interactive build, test, formatter, linter, or locally cached package ",
            "installation is available. Every command requires user approval, has no ",
            "network or stdin, and must use structured arguments rather than shell syntax. ",
            "Do not answer with commands or instructions for the user to run, and never ",
            "claim that you lack filesystem or command access when the corresponding tool ",
            "is available. Paths and command working directories should be relative to the ",
            "workspace root. Keep the requested outcome in focus, reuse prior results, and ",
            "do not repeat an identical read or command unless a mutation changed its inputs. ",
            "Inspect existing files before editing them, but create explicitly requested new ",
            "files or directories directly. Use create_directory for requested folders and ",
            "create_file for project files; do not substitute shell instructions when command ",
            "execution is unavailable. Each run has a guarded safety ceiling of 64 continuation rounds ",
            "and 256 requested tool calls. Never repeat an identical non-empty tool-call batch ",
            "without making progress. Do not request Git operations, deletion, renaming, ",
            "background processes, or network access.",
        ),
        root.display()
    )
}

fn title_from_prompt(prompt: &str) -> String {
    conversation_title::fallback_from_prompt(prompt, "New conversation")
}
