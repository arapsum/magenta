mod commands;
mod stream;
#[cfg(test)]
mod tool_tests;
mod tools;

use std::sync::Arc;

use async_channel::Sender;
use magenta_core::{
    AgentApprovalDecision, AgentProvider, AgentRequest, AgentRunStream, BeginTurn, Conversation,
    ConversationId, ConversationMode, ConversationStore, GenerationConfig, Message, ProviderId,
    WorkspaceAccess, WorkspaceCommandRunner, estimate_agent_overhead,
};

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
    pub store: Arc<dyn ConversationStore>,
    pub workspace: Arc<dyn WorkspaceAccess>,
    pub command_runner: Option<Arc<dyn WorkspaceCommandRunner>>,
    pub root: std::path::PathBuf,
    pub conversation: Conversation,
    pub assistant_message: Message,
    pub run_id: Option<magenta_core::AgentRunId>,
}

#[derive(Clone)]
pub struct AgentApprovalController {
    sender: Sender<ApprovalResponse>,
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
        }
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

        let instructions = agent_instructions(&input.workspace_root);
        let tools = tools::tool_definitions(self.command_runner.is_some());
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

        let (sender, receiver) = async_channel::unbounded();
        let controller = AgentApprovalController { sender };
        let request = AgentRequest {
            generation: prepared.conversation.generation.clone(),
            messages: prepared.context,
            instructions,
            tools,
        };
        let stream = stream::agent_stream(
            AgentStreamContext {
                provider: self.provider.clone(),
                store: self.store.clone(),
                workspace: self.workspace.clone(),
                command_runner: self.command_runner.clone(),
                root: input.workspace_root,
                conversation: prepared.conversation.clone(),
                assistant_message: prepared.assistant_message.clone(),
                run_id: prepared.agent_run_id,
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

        let instructions = agent_instructions(&workspace_root);
        let tools = tools::tool_definitions(self.command_runner.is_some());
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
        let (sender, receiver) = async_channel::unbounded();
        let controller = AgentApprovalController { sender };
        let request = AgentRequest {
            generation: prepared.conversation.generation.clone(),
            messages: prepared.context,
            instructions,
            tools,
        };
        let stream = stream::agent_stream(
            AgentStreamContext {
                provider: self.provider.clone(),
                store: self.store.clone(),
                workspace: self.workspace.clone(),
                command_runner: self.command_runner.clone(),
                root: workspace_root,
                conversation: prepared.conversation.clone(),
                assistant_message: prepared.assistant_message.clone(),
                run_id: prepared.agent_run_id,
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
        "You are Magenta's constrained workspace execution agent operating in {}. You have actual access to this workspace through the supplied tools. You MUST use those tools to inspect, create, or edit files when the user asks for a workspace change. Use run_command after edits when a relevant non-interactive build, test, formatter, linter, or locally cached package installation is available. Every command requires user approval, has no network or stdin, and must use structured arguments rather than shell syntax. Do not answer with commands or instructions for the user to run, and never claim that you lack filesystem or command access when the corresponding tool is available. Paths and command working directories should be relative to the workspace root. Keep the requested outcome in focus, reuse prior results, and do not repeat an identical read or command unless a mutation changed its inputs. Inspect existing files before editing them, but create explicitly requested new files directly. Each run has a guarded safety ceiling of 64 continuation rounds and 256 requested tool calls. Never repeat an identical non-empty tool-call batch without making progress. Do not request Git operations, deletion, renaming, background processes, or network access.",
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
