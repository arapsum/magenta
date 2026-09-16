#![allow(dead_code)]

use futures_util::StreamExt as _;
use magenta_application::PendingAgentGeneration;
use magenta_core::{
    AgentApprovalDecision, AgentApprovalSubject, AgentRunEvent, AgentRunStream, AgentToolOutput,
    AssistantTrace, AssistantTraceEntry, AssistantTraceKind, AssistantTraceStatus, MessageId,
    ProviderError, ProviderId, Timestamp, WorkspaceCommandOutputStream, WorkspaceCommandResult,
};

use super::*;

mod lifecycle;
mod trace;

enum AgentStreamControl {
    Continue,
    Completed(GenerationOutcome),
}

struct ToolTraceUpdate {
    call_id: String,
    tool_name: String,
    status: AssistantTraceStatus,
    input: Option<String>,
    output: Option<String>,
}
