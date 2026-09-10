use std::{
    path::Path,
    sync::{Arc, Mutex},
};

use async_channel::Receiver;
use magenta_core::{
    AgentActivity, AgentActivityKind, AgentApprovalDecision, AgentApprovalRequest,
    AgentApprovalSubject, AgentRunEvent, AgentRunStream, AgentToolCall, AgentToolDefinition,
    AgentToolOutput, AgentWorkspaceChange, Conversation, ConversationId, ConversationStore,
    ProviderId, WorkspaceAccess, WorkspaceChangeKind, WorkspaceChangeState, WorkspaceOperation,
    WorkspacePreview,
};

use super::{AgentStreamContext, ApprovalResponse, agent_error};

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
            if call.name == "run_command" {
                let mut events = super::commands::execute_command(
                    context.clone(),
                    call,
                    approvals.clone(),
                    provider_id.clone(),
                );
                while let Some(event) = futures_util::StreamExt::next(&mut events).await {
                    yield event?;
                }
                continue;
            }
            let prepared = match prepare_tool(
                &context.store,
                &context.workspace,
                &context.root,
                &context.conversation,
                &context.assistant_message,
                context.run_id,
                &call,
            )
            .await {
                Ok(prepared) => prepared,
                Err(error) => {
                    let output = record_failure(&context, &call, &error, &provider_id).await?;
                    yield AgentRunEvent::ToolResult(output);
                    continue;
                }
            };
            let mut preview = prepared.preview;
            let proposed_change = workspace_change(&call, &preview, WorkspaceChangeState::Proposed);
            if let Some(change) = proposed_change.clone() {
                yield AgentRunEvent::WorkspaceChange(change);
            }
            if prepared.operation.is_mutating() || preview.protected {
                let approval = workspace_approval(&call, &prepared.operation, &preview);
                let granted_for_run = approval.can_approve_for_run
                    && permissions
                        .lock()
                        .is_ok_and(|permissions| permissions.approve_workspace_edits);
                if !granted_for_run {
                    record_workspace_approval(&context, &call, &approval, &provider_id).await?;
                    yield AgentRunEvent::ApprovalRequired(approval.clone());
                    let decision = await_decision(&approvals, &approval.request_id).await;
                    if decision == AgentApprovalDecision::Reject {
                        let (change, output) = reject_workspace_tool(
                            &context,
                            &call,
                            proposed_change,
                            &provider_id,
                        )
                        .await?;
                        if let Some(change) = change {
                            yield AgentRunEvent::WorkspaceChange(change);
                        }
                        yield AgentRunEvent::ToolResult(output);
                        continue;
                    }
                    if decision == AgentApprovalDecision::ApproveWorkspaceEditsForRun
                        && approval.can_approve_for_run
                        && let Ok(mut permissions) = permissions.lock()
                    {
                        permissions.approve_workspace_edits = true;
                    }
                }
                if preview.protected {
                    preview = match context
                        .workspace
                        .prepare(context.root.clone(), prepared.operation.clone(), true)
                        .await
                    {
                        Ok(preview) => preview,
                        Err(error) => {
                            let detail = workspace_error_detail(&error);
                            let output = record_failure(&context, &call, &detail, &provider_id)
                                .await?;
                            yield AgentRunEvent::ToolResult(output);
                            continue;
                        }
                    };
                }
            }
            let (output, change) = finish_tool(&context, &prepared.call, preview)
                .await
                .map_err(|error| agent_error(&provider_id, &error))?;
            if let Some(change) = change {
                yield AgentRunEvent::WorkspaceChange(change);
            }
            yield AgentRunEvent::ToolResult(output);
        }
    })
}

struct PreparedTool {
    call: AgentToolCall,
    operation: WorkspaceOperation,
    preview: WorkspacePreview,
}

async fn record_workspace_approval(
    context: &AgentStreamContext,
    call: &AgentToolCall,
    approval: &AgentApprovalRequest,
    provider_id: &ProviderId,
) -> Result<(), magenta_core::ProviderError> {
    record_approval(
        &context.store,
        context.run_id,
        &context.assistant_message,
        context.conversation.id,
        call,
        approval,
    )
    .await
    .map_err(|error| agent_error(provider_id, &error))
}

async fn reject_workspace_tool(
    context: &AgentStreamContext,
    call: &AgentToolCall,
    proposed_change: Option<AgentWorkspaceChange>,
    provider_id: &ProviderId,
) -> Result<(Option<AgentWorkspaceChange>, AgentToolOutput), magenta_core::ProviderError> {
    let change = proposed_change.map(|mut change| {
        change.state = WorkspaceChangeState::Rejected;
        change
    });
    let output = rejected_output(&call.id, "the user rejected this operation");
    record_result(
        &context.store,
        context.run_id,
        &context.assistant_message,
        context.conversation.id,
        &call.name,
        &output,
    )
    .await
    .map_err(|error| agent_error(provider_id, &error))?;
    Ok((change, output))
}

async fn prepare_tool(
    store: &Arc<dyn ConversationStore>,
    workspace: &Arc<dyn WorkspaceAccess>,
    root: &Path,
    conversation: &Conversation,
    assistant_message: &magenta_core::Message,
    run_id: Option<magenta_core::AgentRunId>,
    call: &AgentToolCall,
) -> Result<PreparedTool, String> {
    let operation = parse_operation(call)?;
    let path = operation.path().to_owned();
    record_activity(
        store,
        run_id,
        assistant_message,
        AgentActivity {
            kind: AgentActivityKind::ToolCall,
            call_id: call.id.clone(),
            tool_name: call.name.clone(),
            status: "requested".to_owned(),
            summary: format!("{} {path}", call.name),
            detail: call.arguments.clone(),
        },
        conversation.id,
    )
    .await?;

    let preview = workspace
        .prepare(root.to_path_buf(), operation.clone(), false)
        .await
        .map_err(|error| workspace_error_detail(&error))?;
    Ok(PreparedTool {
        call: call.clone(),
        operation,
        preview,
    })
}

async fn finish_tool(
    context: &AgentStreamContext,
    call: &AgentToolCall,
    preview: WorkspacePreview,
) -> Result<(AgentToolOutput, Option<AgentWorkspaceChange>), String> {
    let proposed = workspace_change(call, &preview, WorkspaceChangeState::Proposed);
    let output = if let Some(mutation) = preview.mutation {
        match context
            .workspace
            .commit(context.root.clone(), mutation)
            .await
        {
            Ok(result) => AgentToolOutput {
                call_id: call.id.clone(),
                output: result,
                is_error: false,
            },
            Err(error) => failed_output(&call.id, &workspace_error_detail(&error)),
        }
    } else {
        AgentToolOutput {
            call_id: call.id.clone(),
            output: preview.output,
            is_error: false,
        }
    };
    record_result(
        &context.store,
        context.run_id,
        &context.assistant_message,
        context.conversation.id,
        &call.name,
        &output,
    )
    .await?;
    let change = proposed.map(|mut change| {
        if output.is_error {
            change.state = WorkspaceChangeState::Failed;
            change.error = Some(output.output.clone());
        } else {
            change.state = WorkspaceChangeState::Committed;
        }
        change
    });
    Ok((output, change))
}

fn workspace_approval(
    call: &AgentToolCall,
    operation: &WorkspaceOperation,
    preview: &WorkspacePreview,
) -> AgentApprovalRequest {
    AgentApprovalRequest {
        request_id: format!("{}-approval", call.id),
        tool_call_id: call.id.clone(),
        tool_name: call.name.clone(),
        reason: if preview.protected {
            "This path may contain credentials or secrets.".to_owned()
        } else {
            preview.summary.clone()
        },
        subject: AgentApprovalSubject::Workspace {
            path: operation.path().to_owned(),
            diff: preview.diff.clone(),
            protected_read: preview.protected,
        },
        can_approve_for_run: can_approve_for_run(operation, preview),
    }
}

const fn can_approve_for_run(operation: &WorkspaceOperation, preview: &WorkspacePreview) -> bool {
    !preview.protected
        && matches!(
            operation,
            WorkspaceOperation::ApplyPatch { .. } | WorkspaceOperation::CreateFile { .. }
        )
}

fn workspace_change(
    call: &AgentToolCall,
    preview: &WorkspacePreview,
    state: WorkspaceChangeState,
) -> Option<AgentWorkspaceChange> {
    let mutation = preview.mutation.as_ref()?;
    Some(AgentWorkspaceChange {
        call_id: call.id.clone(),
        path: mutation.path.clone(),
        kind: if mutation.creates_file {
            WorkspaceChangeKind::Create
        } else {
            WorkspaceChangeKind::Modify
        },
        content: String::from_utf8(mutation.replacement.clone()).ok()?,
        diff: preview.diff.clone().unwrap_or_default(),
        state,
        error: None,
    })
}

async fn record_failure(
    context: &AgentStreamContext,
    call: &AgentToolCall,
    message: &str,
    provider_id: &ProviderId,
) -> Result<AgentToolOutput, magenta_core::ProviderError> {
    let output = failed_output(&call.id, message);
    record_result(
        &context.store,
        context.run_id,
        &context.assistant_message,
        context.conversation.id,
        &call.name,
        &output,
    )
    .await
    .map_err(|error| agent_error(provider_id, &error))?;
    Ok(output)
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

pub(super) async fn record_approval(
    store: &Arc<dyn ConversationStore>,
    run_id: Option<magenta_core::AgentRunId>,
    assistant_message: &magenta_core::Message,
    conversation_id: ConversationId,
    call: &AgentToolCall,
    request: &AgentApprovalRequest,
) -> Result<(), String> {
    record_activity(
        store,
        run_id,
        assistant_message,
        AgentActivity {
            kind: AgentActivityKind::ApprovalRequested,
            call_id: call.id.clone(),
            tool_name: call.name.clone(),
            status: "awaiting-approval".to_owned(),
            summary: request.reason.clone(),
            detail: match &request.subject {
                AgentApprovalSubject::Workspace { diff, .. } => diff.clone().unwrap_or_default(),
                AgentApprovalSubject::Command(command) => command.display(),
            },
        },
        conversation_id,
    )
    .await
}

pub(super) async fn record_activity(
    store: &Arc<dyn ConversationStore>,
    run_id: Option<magenta_core::AgentRunId>,
    assistant_message: &magenta_core::Message,
    activity: AgentActivity,
    conversation_id: ConversationId,
) -> Result<(), String> {
    let Some(run_id) = run_id else {
        return Ok(());
    };
    store
        .append_agent_activity(magenta_core::AgentActivityRecord {
            run_id,
            assistant_message_id: assistant_message.id,
            conversation_id,
            activity,
        })
        .await
        .map_err(|error| error.to_string())
}

pub(super) async fn record_result(
    store: &Arc<dyn ConversationStore>,
    run_id: Option<magenta_core::AgentRunId>,
    assistant_message: &magenta_core::Message,
    conversation_id: ConversationId,
    tool_name: &str,
    output: &AgentToolOutput,
) -> Result<(), String> {
    record_activity(
        store,
        run_id,
        assistant_message,
        AgentActivity {
            kind: AgentActivityKind::ToolResult,
            call_id: output.call_id.clone(),
            tool_name: tool_name.to_owned(),
            status: if output.is_error {
                "failed".to_owned()
            } else {
                "completed".to_owned()
            },
            summary: tool_name.to_owned(),
            detail: output.output.clone(),
        },
        conversation_id,
    )
    .await
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
    let mut definitions = vec![
        definition(
            "list_files",
            "List workspace files and directories. Paths are relative; use '.' for the root.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string"},
                    "depth": {"type": "integer", "minimum": 0, "maximum": 6}
                },
                "required": ["path", "depth"],
                "additionalProperties": false
            }),
            false,
            false,
        ),
        definition(
            "search_text",
            "Find literal text in ignored-aware UTF-8 workspace files. Use '.' for the root.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "query": {"type": "string"},
                    "path": {"type": "string"},
                    "glob": {"type": ["string", "null"]}
                },
                "required": ["query", "path", "glob"],
                "additionalProperties": false
            }),
            false,
            false,
        ),
        definition(
            "read_file",
            "Read a bounded range from one UTF-8 workspace file.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string"},
                    "start_line": {"type": ["integer", "null"]},
                    "line_count": {"type": ["integer", "null"]}
                },
                "required": ["path", "start_line", "line_count"],
                "additionalProperties": false
            }),
            false,
            true,
        ),
        definition(
            "apply_patch",
            "Propose a unified patch for one existing UTF-8 workspace file.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string"},
                    "unified_diff": {"type": "string"}
                },
                "required": ["path", "unified_diff"],
                "additionalProperties": false
            }),
            true,
            false,
        ),
        definition(
            "create_file",
            "Propose a new UTF-8 workspace file. It must not already exist.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string"},
                    "content": {"type": "string"}
                },
                "required": ["path", "content"],
                "additionalProperties": false
            }),
            true,
            false,
        ),
    ];
    if commands_available {
        definitions.push(definition(
            "run_command",
            "Run one non-interactive command in the selected workspace after explicit user approval. Arguments are passed directly without shell interpolation; network and stdin are unavailable.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "program": {"type": "string"},
                    "args": {"type": "array", "items": {"type": "string"}, "maxItems": 64},
                    "cwd": {"type": "string"},
                    "timeout_seconds": {"type": "integer", "minimum": 1, "maximum": 600}
                },
                "required": ["program", "args", "cwd", "timeout_seconds"],
                "additionalProperties": false
            }),
            true,
            false,
        ));
    }
    definitions
}

fn definition(
    name: &str,
    description: &str,
    parameters: serde_json::Value,
    mutating: bool,
    protected_read: bool,
) -> AgentToolDefinition {
    AgentToolDefinition {
        name: name.to_owned(),
        description: description.to_owned(),
        parameters,
        mutating,
        protected_read,
    }
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
mod tests {
    use super::*;

    fn preview(protected: bool) -> WorkspacePreview {
        WorkspacePreview {
            path: "src/lib.rs".to_owned(),
            summary: "update file".to_owned(),
            output: String::new(),
            diff: None,
            protected,
            mutation: None,
        }
    }

    #[test]
    fn run_permission_is_limited_to_unprotected_file_edits() {
        assert!(can_approve_for_run(
            &WorkspaceOperation::ApplyPatch {
                path: "src/lib.rs".to_owned(),
                unified_diff: String::new(),
            },
            &preview(false),
        ));
        assert!(can_approve_for_run(
            &WorkspaceOperation::CreateFile {
                path: "src/new.rs".to_owned(),
                content: String::new(),
            },
            &preview(false),
        ));
        assert!(!can_approve_for_run(
            &WorkspaceOperation::ReadFile {
                path: "src/lib.rs".to_owned(),
                start_line: None,
                line_count: None,
            },
            &preview(false),
        ));
        assert!(!can_approve_for_run(
            &WorkspaceOperation::ApplyPatch {
                path: "src/lib.rs".to_owned(),
                unified_diff: String::new(),
            },
            &preview(true),
        ));
    }
}
