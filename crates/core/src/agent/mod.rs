use std::pin::Pin;

use futures_core::Stream;
use serde::{Deserialize, Serialize};

use crate::{
    AgentRunId, ConversationId, EffortLevel, GenerationConfig, GenerationOutcome, Message,
    MessageId, ModelId, ProviderError, ProviderId, WorkspaceCommand, WorkspaceCommandOutputStream,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentRequest {
    pub generation: GenerationConfig,
    pub messages: Vec<Message>,
    pub instructions: String,
    pub tools: Vec<AgentToolDefinition>,
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
    Reject,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentRunEvent {
    Started,
    TextDelta(String),
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AgentActivityKind {
    ToolCall,
    ApprovalRequested,
    ToolResult,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentActivity {
    pub kind: AgentActivityKind,
    pub call_id: String,
    pub tool_name: String,
    pub status: String,
    pub summary: String,
    pub detail: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentActivityRecord {
    pub run_id: AgentRunId,
    pub assistant_message_id: MessageId,
    pub conversation_id: ConversationId,
    pub activity: AgentActivity,
}
