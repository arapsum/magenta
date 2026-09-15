mod data;

pub use data::{
    AgentContentCache, AgentDataFuture, AgentMemory, AgentMemoryStore, AgentSession,
    AgentSessionState, AgentSessionStore, CachedContent, CodeChunk, CodeIndex, CodeIndexMaintainer,
    CodeIndexReport, CodeMatch, EmbeddingProvider, MemoryKind, MemoryMatch, MemoryState,
    NewAgentMemory, RetrievedContextBlock, RetrievedContextKind,
};

use std::pin::Pin;

use futures_core::Stream;
use serde::{Deserialize, Serialize};

use crate::{
    EffortLevel, GenerationConfig, GenerationOutcome, Message, ModelId, ProviderError, ProviderId,
    Timestamp, WorkspaceCommand, WorkspaceCommandOutputStream,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssistantTextPhase {
    Commentary,
    FinalAnswer,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssistantTraceKind {
    ReasoningSummary,
    Tool,
}

pub type AssistantTraceEntryKind = AssistantTraceKind;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssistantTraceStatus {
    Streaming,
    Requested,
    Running,
    AwaitingApproval,
    Completed,
    Rejected,
    Failed,
    Stopped,
}

pub type AssistantTraceEntryStatus = AssistantTraceStatus;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AssistantTrace {
    pub entries: Vec<AssistantTraceEntry>,
    pub thinking_duration_ms: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssistantTraceEntry {
    pub key: String,
    pub sequence: u64,
    pub kind: AssistantTraceKind,
    pub status: AssistantTraceStatus,
    pub title: String,
    pub tool_name: Option<String>,
    pub input: String,
    pub output: String,
    pub started_at: Option<Timestamp>,
    pub finished_at: Option<Timestamp>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AgentRequest {
    pub generation: GenerationConfig,
    pub messages: Vec<Message>,
    pub instructions: String,
    pub tools: Vec<AgentToolDefinition>,
    /// Ranked, project-scoped context selected independently from chat history.
    pub retrieved_context: Vec<RetrievedContextBlock>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentResumeRequest {
    pub continuation: AgentContinuation,
    pub outputs: Vec<AgentToolOutput>,
    pub instructions: String,
    pub tools: Vec<AgentToolDefinition>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentContinuation {
    pub provider: ProviderId,
    pub model: ModelId,
    pub effort: EffortLevel,
    pub payload: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
    pub mutating: bool,
    pub protected_read: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentToolOutput {
    pub call_id: String,
    pub output: String,
    pub is_error: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentProviderEvent {
    Started,
    TextDelta(String),
    TextDeltaWithPhase {
        delta: String,
        phase: AssistantTextPhase,
    },
    ReasoningSummaryStarted {
        key: String,
        title: String,
    },
    ReasoningSummaryDelta {
        key: String,
        delta: String,
    },
    ReasoningSummaryCompleted {
        key: String,
        text: String,
    },
    ToolCall {
        call: AgentToolCall,
        continuation: AgentContinuation,
    },
    Completed(GenerationOutcome),
}

pub type AgentProviderStream =
    Pin<Box<dyn Stream<Item = Result<AgentProviderEvent, ProviderError>> + Send + 'static>>;

pub trait AgentProvider: Send + Sync {
    fn start(&self, request: AgentRequest) -> AgentProviderStream;

    fn resume(&self, request: AgentResumeRequest) -> AgentProviderStream;
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ConversationMode {
    #[default]
    Chat,
    Agent,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentApprovalRequest {
    pub request_id: String,
    pub tool_call_id: String,
    pub tool_name: String,
    pub reason: String,
    pub subject: AgentApprovalSubject,
    pub can_approve_for_run: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentApprovalSubject {
    Workspace {
        path: String,
        diff: Option<String>,
        protected_read: bool,
    },
    Command(WorkspaceCommand),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentApprovalDecision {
    Approve,
    ApproveWorkspaceEditsForRun,
    Reject,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentRunEvent {
    Started,
    TextDelta(String),
    TextDeltaWithPhase {
        delta: String,
        phase: AssistantTextPhase,
    },
    ReasoningSummaryStarted {
        key: String,
        title: String,
    },
    ReasoningSummaryDelta {
        key: String,
        delta: String,
    },
    ReasoningSummaryCompleted {
        key: String,
        text: String,
    },
    ToolCall(AgentToolCall),
    ToolResult(AgentToolOutput),
    WorkspaceChange(AgentWorkspaceChange),
    CommandStarted {
        call_id: String,
        command: WorkspaceCommand,
    },
    CommandOutput {
        call_id: String,
        stream: WorkspaceCommandOutputStream,
        chunk: String,
    },
    WorkspaceInvalidated,
    ApprovalRequired(AgentApprovalRequest),
    Completed(GenerationOutcome),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkspaceChangeKind {
    Create,
    Modify,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkspaceChangeState {
    Proposed,
    Staged,
    Committed,
    Rejected,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentWorkspaceChange {
    pub call_id: String,
    pub path: String,
    pub kind: WorkspaceChangeKind,
    pub content: String,
    pub diff: String,
    pub state: WorkspaceChangeState,
    pub error: Option<String>,
}

pub type AgentRunStream =
    Pin<Box<dyn Stream<Item = Result<AgentRunEvent, ProviderError>> + Send + 'static>>;
