use std::{
    path::Path,
    sync::{Arc, Mutex},
};

use async_channel::Receiver;
use magenta_core::{
    AgentApprovalDecision, AgentApprovalRequest, AgentApprovalSubject, AgentRunEvent,
    AgentRunStream, AgentToolCall, AgentToolDefinition, AgentToolOutput, AgentWorkspaceChange,
    MemoryKind, MemoryState, NewAgentMemory, ProviderId, WorkspaceAccess, WorkspaceChangeKind,
    WorkspaceChangeState, WorkspaceOperation, WorkspacePreview,
};
use sha2::{Digest as _, Sha256};

use super::{AgentContextServices, AgentStreamContext, ApprovalResponse, agent_error};

mod commands;
mod data;
mod definition;
mod workspace;

#[cfg(test)]
use workspace::can_approve_for_run;

#[derive(Default)]
pub(super) struct AgentRunPermissions {
    approve_workspace_edits: bool,
}

pub(super) type AgentRunPermissionsHandle = Arc<Mutex<AgentRunPermissions>>;

pub fn execute_tools(
    context: AgentStreamContext,
    calls: Vec<AgentToolCall>,
    approvals: Receiver<ApprovalResponse>,
    provider_id: ProviderId,
    permissions: AgentRunPermissionsHandle,
) -> AgentRunStream {
    Box::pin(async_stream::try_stream! {
        for call in calls {
            if matches!(call.name.as_str(), "search_code" | "save_memory_candidate") {
                let output = data::execute_data_tool(&context, &call).await;
                yield AgentRunEvent::ToolResult(output);
                continue;
            }

            if call.name == "run_command" {
                let mut events = commands::execute_command(
                    context.clone(),
                    call,
                    approvals.clone(),
                );
                while let Some(event) = futures_util::StreamExt::next(&mut events).await {
                    yield event?;
                }

                continue;
            }

            let mut events = workspace::execute_workspace_tool(
                context.clone(),
                call,
                approvals.clone(),
                provider_id.clone(),
                permissions.clone(),
            );
            while let Some(event) = futures_util::StreamExt::next(&mut events).await {
                yield event?;
            }
        }
    })
}

pub(super) async fn await_decision(
    approvals: &Receiver<ApprovalResponse>,
    request_id: &str,
) -> AgentApprovalDecision {
    while let Ok(response) = approvals.recv().await {
        if response.request_id == request_id {
            return response.decision;
        }
    }
    AgentApprovalDecision::Reject
}

pub fn parse_operation(call: &AgentToolCall) -> Result<WorkspaceOperation, String> {
    let value: serde_json::Value = serde_json::from_str(&call.arguments)
        .map_err(|error| format!("invalid tool arguments: {error}"))?;

    let string = |name: &str| {
        value
            .get(name)
            .and_then(serde_json::Value::as_str)
            .map(ToOwned::to_owned)
            .ok_or_else(|| format!("tool argument {name} must be a string"))
    };

    let path_or_root = |name: &str| {
        string(name).map(|path| {
            if path.is_empty() {
                ".".to_owned()
            } else {
                path
            }
        })
    };

    match call.name.as_str() {
        "list_files" => Ok(WorkspaceOperation::ListFiles {
            path: path_or_root("path")?,
            depth: value
                .get("depth")
                .and_then(serde_json::Value::as_u64)
                .ok_or_else(|| "tool argument depth must be an integer".to_owned())?
                .try_into()
                .map_err(|_| "tool argument depth is too large".to_owned())?,
        }),
        "search_text" => Ok(WorkspaceOperation::SearchText {
            query: string("query")?,
            path: path_or_root("path")?,
            glob: value
                .get("glob")
                .and_then(serde_json::Value::as_str)
                .map(ToOwned::to_owned),
        }),
        "read_file" => Ok(WorkspaceOperation::ReadFile {
            path: string("path")?,
            start_line: optional_u32(&value, "start_line")?,
            line_count: optional_u32(&value, "line_count")?,
        }),
        "apply_patch" => Ok(WorkspaceOperation::ApplyPatch {
            path: string("path")?,
            unified_diff: string("unified_diff")?,
        }),
        "create_file" => Ok(WorkspaceOperation::CreateFile {
            path: string("path")?,
            content: string("content")?,
        }),
        "create_directory" => Ok(WorkspaceOperation::CreateDirectory {
            path: string("path")?,
        }),
        name => Err(format!("unknown workspace tool {name}")),
    }
}

fn optional_u32(value: &serde_json::Value, name: &str) -> Result<Option<u32>, String> {
    value
        .get(name)
        .ok_or_else(|| format!("tool argument {name} is required"))?
        .as_u64()
        .map(|number| {
            u32::try_from(number).map_err(|_| format!("tool argument {name} is too large"))
        })
        .transpose()
}

fn workspace_error_detail(error: &magenta_core::WorkspaceError) -> String {
    error.source.to_string()
}

pub fn tool_definitions(commands_available: bool) -> Vec<AgentToolDefinition> {
    definition::tool_definitions(commands_available)
}

pub(super) fn rejected_output(call_id: &str, message: &str) -> AgentToolOutput {
    AgentToolOutput {
        call_id: call_id.to_owned(),
        output: message.to_owned(),
        is_error: true,
    }
}

pub(super) fn failed_output(call_id: &str, message: &str) -> AgentToolOutput {
    AgentToolOutput {
        call_id: call_id.to_owned(),
        output: message.to_owned(),
        is_error: true,
    }
}

#[cfg(test)]
#[path = "../../../test/agent/tool/mod.rs"]
mod tests;
