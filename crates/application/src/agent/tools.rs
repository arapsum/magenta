use std::{path::Path, sync::Arc};

use async_channel::Receiver;
use magenta_core::{
    AgentActivity, AgentActivityKind, AgentApprovalDecision, AgentApprovalRequest, AgentRunEvent,
    AgentRunStream, AgentToolCall, AgentToolDefinition, AgentToolOutput, Conversation,
    ConversationId, ConversationStore, ProviderId, WorkspaceAccess, WorkspaceOperation,
    WorkspacePreview,
};

use super::{AgentStreamContext, ApprovalResponse, agent_error};

pub fn execute_tools(
    context: AgentStreamContext,
    calls: Vec<AgentToolCall>,
    approvals: Receiver<ApprovalResponse>,
    provider_id: ProviderId,
) -> AgentRunStream {
    Box::pin(async_stream::try_stream! {
        for call in calls {
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
            if prepared.operation.is_mutating() || preview.protected {
                let approval = AgentApprovalRequest {
                    request_id: format!("{}-approval", call.id),
                    tool_call_id: call.id.clone(),
                    tool_name: call.name.clone(),
                    path: prepared.operation.path().to_owned(),
                    reason: if preview.protected {
                        "This path may contain credentials or secrets.".to_owned()
                    } else {
                        preview.summary.clone()
                    },
                    diff: preview.diff.clone(),
                    protected_read: preview.protected,
                };
                record_approval(
                    &context.store,
                    context.run_id,
                    &context.assistant_message,
                    context.conversation.id,
                    &call,
                    &approval,
                )
                .await
                .map_err(|error| agent_error(&provider_id, &error))?;
                yield AgentRunEvent::ApprovalRequired(approval.clone());
                let decision = await_decision(&approvals, &approval.request_id).await;
                if decision != AgentApprovalDecision::Approve {
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
                    .map_err(|error| agent_error(&provider_id, &error))?;
                    yield AgentRunEvent::ToolResult(output);
                    continue;
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
            let output = finish_tool(&context, &prepared.call, preview)
                .await
                .map_err(|error| agent_error(&provider_id, &error))?;
            yield AgentRunEvent::ToolResult(output);
        }
    })
}

struct PreparedTool {
    call: AgentToolCall,
    operation: WorkspaceOperation,
    preview: WorkspacePreview,
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
) -> Result<AgentToolOutput, String> {
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
    Ok(output)
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

async fn await_decision(
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

async fn record_approval(
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
            detail: request.diff.clone().unwrap_or_default(),
        },
        conversation_id,
    )
    .await
}

async fn record_activity(
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

async fn record_result(
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

pub fn tool_definitions() -> Vec<AgentToolDefinition> {
    vec![
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
    ]
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

fn rejected_output(call_id: &str, message: &str) -> AgentToolOutput {
    AgentToolOutput {
        call_id: call_id.to_owned(),
        output: message.to_owned(),
        is_error: true,
    }
}

fn failed_output(call_id: &str, message: &str) -> AgentToolOutput {
    AgentToolOutput {
        call_id: call_id.to_owned(),
        output: message.to_owned(),
        is_error: true,
    }
}
