mod stream;
mod tool;

use std::sync::Arc;

use async_channel::Sender;
use magenta_core::{
    AgentApprovalDecision, AgentContentCache, AgentMemoryStore, AgentProvider, AgentRequest,
    AgentRunStream, AgentSession, AgentSessionState, AgentSessionStore, BeginTurn, CodeIndex,
    CodeIndexMaintainer, Conversation, ConversationId, ConversationMode, ConversationStore,
    EmbeddingProvider, GenerationConfig, MemoryKind, MemoryState, Message, NewAgentMemory,
    PreparedTurn, ProviderId, RetrievedContextBlock, RetrievedContextKind, WorkspaceAccess,
    WorkspaceCommandRunner, WorkspaceSessionAccess, estimate_agent_overhead,
};

use super::trace::AssistantTraceRecorder;
use crate::{RetryMessageError, SendMessageError};

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
    pub const fn has_pending_workspace_review(&self) -> bool {
        self.review.is_some()
    }

    #[must_use]
    pub fn is_workspace_review_for(&self, message_id: magenta_core::MessageId) -> bool {
        self.review
            .as_ref()
            .is_some_and(|review| review.assistant_message_id == message_id)
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
}

impl AgentReviewHandle {
    pub(crate) async fn awaiting_review(&self) {
        let _ = self
            .store
            .set_session_state(
                self.root.clone(),
                self.session_id.clone(),
                AgentSessionState::AwaitingReview,
            )
            .await;
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

impl RunWorkspaceAgent {
    #[must_use]
    pub fn new(
        provider: Arc<dyn AgentProvider>,
        store: Arc<dyn ConversationStore>,
        workspace: Arc<dyn WorkspaceAccess>,
        command_runner: Option<Arc<dyn WorkspaceCommandRunner>>,
    ) -> Self {
        Self {
            provider,
            store,
            workspace,
            command_runner,
            context_services: None,
            code_indexer: None,
            workspace_sessions: None,
        }
    }

    #[must_use]
    pub fn with_code_indexer(mut self, indexer: Arc<dyn CodeIndexMaintainer>) -> Self {
        self.code_indexer = Some(indexer);
        self
    }

    #[must_use]
    pub fn with_workspace_sessions(
        mut self,
        workspace: Arc<dyn WorkspaceSessionAccess>,
        store: Arc<dyn AgentSessionStore>,
    ) -> Self {
        self.workspace_sessions = Some((workspace, store));
        self
    }

    /// Enables project memory and semantic code retrieval for subsequent runs.
    #[must_use]
    pub fn with_agent_context(
        mut self,
        memories: Arc<dyn AgentMemoryStore>,
        code: Arc<dyn CodeIndex>,
        embeddings: Arc<dyn EmbeddingProvider>,
        cache: Arc<dyn AgentContentCache>,
    ) -> Self {
        self.context_services = Some(AgentContextServices {
            memories,
            code,
            embeddings,
            cache,
        });
        self
    }

    #[must_use]
    pub const fn supports_commands(&self) -> bool {
        self.command_runner.is_some()
    }

    /// Persists an agent turn before beginning provider work.
    ///
    /// # Errors
    /// Returns an error for an empty prompt, missing workspace, or persistence failure.
    pub async fn execute(
        &self,
        input: RunWorkspaceAgentInput,
    ) -> Result<PendingAgentGeneration, SendMessageError> {
        let prompt = input.prompt.trim().to_owned();
        if prompt.is_empty() {
            return Err(SendMessageError::EmptyPrompt);
        }
        if !input.workspace_root.is_dir() {
            return Err(SendMessageError::WorkspaceUnavailable);
        }

        self.prepare_agent_context(&input.workspace_root, &prompt)
            .await;

        let retrieved_context = self
            .retrieve_context(&input.workspace_root, &prompt, None)
            .await;

        let instructions = instructions_with_context(
            agent_instructions(&input.workspace_root),
            &retrieved_context,
        );

        let tools = tool::tool_definitions(self.command_runner.is_some());

        let request_overhead_tokens = estimate_agent_overhead(&instructions, &tools);

        let prepared = self
            .store
            .begin_turn(BeginTurn {
                conversation_id: match input.target {
                    AgentSendTarget::New => None,
                    AgentSendTarget::Existing(id) => Some(id),
                },
                title: title_from_prompt(&prompt),
                prompt,
                attachments: Vec::new(),
                generation: input.generation,
                mode: ConversationMode::Agent,
                workspace_root: Some(input.workspace_root.clone()),
                request_overhead_tokens,
            })
            .await?;

        let review = self.start_review(&input.workspace_root, &prepared).await;
        let (sender, receiver) = async_channel::unbounded();

        let controller = AgentApprovalController {
            sender,
            review: review.clone(),
        };

        let request = AgentRequest {
            generation: prepared.conversation.generation.clone(),
            messages: prepared.context,
            instructions,
            tools,
            retrieved_context,
        };

        let stream = stream::agent_stream(
            AgentStreamContext {
                provider: self.provider.clone(),
                workspace: self.workspace.clone(),
                command_runner: self.command_runner.clone(),
                root: input.workspace_root,
                conversation: prepared.conversation.clone(),
                trace: AssistantTraceRecorder::new(
                    self.store.clone(),
                    prepared.assistant_message.id,
                    prepared.assistant_message.assistant_trace.clone(),
                ),
                review,
                context_services: self.context_services.clone(),
            },
            request,
            receiver,
        );

        Ok(PendingAgentGeneration {
            conversation: prepared.conversation,
            user_message: prepared.user_message,
            assistant_message: prepared.assistant_message,
            stream,
            controller,
            user_sequence: prepared.user_sequence,
            assistant_sequence: prepared.assistant_sequence,
            context_report: prepared.context_report,
        })
    }

    async fn prepare_agent_context(&self, root: &std::path::Path, prompt: &str) {
        if let Some(content) = prompt.strip_prefix("/remember ")
            && let Some(services) = &self.context_services
        {
            let embedding = services
                .embeddings
                .embed(vec![content.to_owned()])
                .await
                .ok()
                .and_then(|mut values| values.pop());

            let _ = services
                .memories
                .remember(
                    root.to_path_buf(),
                    NewAgentMemory {
                        kind: MemoryKind::Fact,
                        state: MemoryState::Active,
                        content: content.to_owned(),
                        source_conversation_id: None,
                        confidence: 1.0,
                        embedding,
                    },
                )
                .await;
        }

        if let Some(indexer) = &self.code_indexer {
            let _ = indexer.refresh(root.to_path_buf()).await;
        }
    }

    /// Starts a new agent attempt while retaining the failed response. The
    /// storage port excludes that failed assistant from the provider context.
    ///
    /// # Errors
    ///
    /// Returns an error when the target conversation, workspace, or persisted
    /// retry turn is unavailable.
    pub async fn retry(
        &self,
        input: RetryWorkspaceAgentInput,
    ) -> Result<PendingAgentGeneration, RetryMessageError> {
        let loaded = self.store.load(input.conversation_id).await?;
        if loaded.conversation.mode != ConversationMode::Agent {
            return Err(RetryMessageError::AgentContinuation);
        }
        let workspace_root = loaded
            .conversation
            .workspace_root
            .clone()
            .ok_or(RetryMessageError::WorkspaceUnavailable)?;
        if !workspace_root.is_dir() {
            return Err(RetryMessageError::WorkspaceUnavailable);
        }

        if let Some(indexer) = &self.code_indexer {
            let _ = indexer.refresh(workspace_root.clone()).await;
        }

        let instructions = agent_instructions(&workspace_root);

        let tools = tool::tool_definitions(self.command_runner.is_some());

        let request_overhead_tokens = estimate_agent_overhead(&instructions, &tools);

        let prepared = self
            .store
            .begin_retry(
                input.conversation_id,
                input.target_message_id,
                input.generation,
                request_overhead_tokens,
            )
            .await?;

        let retrieved_context = self
            .retrieve_context(&workspace_root, &prepared.user_message.content, None)
            .await;

        let instructions = instructions_with_context(instructions, &retrieved_context);

        let review = self.start_review(&workspace_root, &prepared).await;
        let (sender, receiver) = async_channel::unbounded();

        let controller = AgentApprovalController {
            sender,
            review: review.clone(),
        };

        let request = AgentRequest {
            generation: prepared.conversation.generation.clone(),
            messages: prepared.context,
            instructions,
            tools,
            retrieved_context,
        };

        let stream = stream::agent_stream(
            AgentStreamContext {
                provider: self.provider.clone(),
                workspace: self.workspace.clone(),
                command_runner: self.command_runner.clone(),
                root: workspace_root,
                conversation: prepared.conversation.clone(),
                trace: AssistantTraceRecorder::new(
                    self.store.clone(),
                    prepared.assistant_message.id,
                    prepared.assistant_message.assistant_trace.clone(),
                ),
                review,
                context_services: self.context_services.clone(),
            },
            request,
            receiver,
        );

        Ok(PendingAgentGeneration {
            conversation: prepared.conversation,
            user_message: prepared.user_message,
            assistant_message: prepared.assistant_message,
            stream,
            controller,
            user_sequence: prepared.user_sequence,
            assistant_sequence: prepared.assistant_sequence,
            context_report: prepared.context_report,
        })
    }

    async fn retrieve_context(
        &self,
        root: &std::path::Path,
        query: &str,
        session_id: Option<String>,
    ) -> Vec<RetrievedContextBlock> {
        let Some(services) = &self.context_services else {
            return Vec::new();
        };
        let embedding = services
            .embeddings
            .embed(vec![query.to_owned()])
            .await
            .ok()
            .and_then(|mut values| values.pop());

        let (memories, code) = futures_util::join!(
            services
                .memories
                .recall(root.to_path_buf(), query.to_owned(), embedding.clone(), 8),
            services.code.search_code(
                root.to_path_buf(),
                query.to_owned(),
                embedding,
                session_id,
                8
            ),
        );

        let mut blocks = Vec::new();
        if let Ok(memories) = memories {
            blocks.extend(memories.into_iter().map(|item| RetrievedContextBlock {
                kind: RetrievedContextKind::Memory,
                source: format!("memory:{}", item.memory.id),
                content: item.memory.content,
                score: item.score,
            }));
        }

        if let Ok(code) = code {
            blocks.extend(code.into_iter().map(|item| RetrievedContextBlock {
                kind: RetrievedContextKind::Code,
                source: format!(
                    "{}:{}-{}",
                    item.chunk.path, item.chunk.start_line, item.chunk.end_line
                ),
                content: item.chunk.content,
                score: item.score,
            }));
        }

        blocks.sort_by(|left, right| {
            right
                .score
                .partial_cmp(&left.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let mut bytes = 0_usize;
        blocks.retain(|block| {
            let next = bytes
                .saturating_add(block.content.len())
                .saturating_add(block.source.len());
            if next > 4_500 {
                false
            } else {
                bytes = next;
                true
            }
        });
        blocks
    }

    async fn start_review(
        &self,
        root: &std::path::Path,
        prepared: &PreparedTurn,
    ) -> Option<AgentReviewHandle> {
        let (workspace, store) = self.workspace_sessions.as_ref()?;
        let session_id = format!(
            "conversation-{}-message-{}",
            prepared.conversation.id.0, prepared.assistant_message.id.0
        );

        let database_path = workspace
            .start_session(root.to_path_buf(), session_id.clone())
            .await
            .ok()?;

        let timestamp = magenta_core::Timestamp(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .ok()?
                .as_millis()
                .try_into()
                .ok()?,
        );

        if store
            .create_session(
                root.to_path_buf(),
                AgentSession {
                    id: session_id.clone(),
                    conversation_id: prepared.conversation.id,
                    agentfs_path: database_path,
                    state: AgentSessionState::Running,
                    created_at: timestamp,
                    updated_at: timestamp,
                },
            )
            .await
            .is_err()
        {
            let _ = workspace
                .discard_session(root.to_path_buf(), session_id.clone())
                .await;
            return None;
        }

        Some(AgentReviewHandle {
            workspace: workspace.clone(),
            store: store.clone(),
            root: root.to_path_buf(),
            session_id,
            assistant_message_id: prepared.assistant_message.id,
        })
    }
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
            "files directly. Each run has a guarded safety ceiling of 64 continuation rounds ",
            "and 256 requested tool calls. Never repeat an identical non-empty tool-call batch ",
            "without making progress. Do not request Git operations, deletion, renaming, ",
            "background processes, or network access.",
        ),
        root.display()
    )
}

fn title_from_prompt(prompt: &str) -> String {
    let title = prompt
        .lines()
        .next()
        .unwrap_or_default()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");

    if title.chars().count() > 46 {
        format!("{}...", title.chars().take(43).collect::<String>())
    } else if title.is_empty() {
        "New conversation".to_owned()
    } else {
        title
    }
}
