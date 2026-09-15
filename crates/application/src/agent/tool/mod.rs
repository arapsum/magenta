use std::{
    path::Path,
    sync::{Arc, Mutex},
};

use async_channel::Receiver;
use magenta_core::{
    AgentActivity, AgentActivityKind, AgentApprovalDecision, AgentApprovalRequest,
    AgentApprovalSubject, AgentRunEvent, AgentRunStream, AgentToolCall, AgentToolDefinition,
    AgentToolOutput, AgentWorkspaceChange, Conversation, ConversationId, ConversationStore,
    MemoryKind, MemoryState, NewAgentMemory, ProviderId, WorkspaceAccess, WorkspaceChangeKind,
    WorkspaceChangeState, WorkspaceOperation, WorkspacePreview,
};
use sha2::{Digest as _, Sha256};

use super::{AgentContextServices, AgentStreamContext, ApprovalResponse, agent_error};

mod commands;
mod definition;

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
                let output = execute_data_tool(&context, &call, &provider_id).await?;
                yield AgentRunEvent::ToolResult(output);
                continue;
            }

            if call.name == "run_command" {
                let mut events = commands::execute_command(
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

            for event in execute_workspace_tool(
                &context,
                call,
                &approvals,
                &provider_id,
                &permissions,
            )
            .await?
            {
                yield event;
            }
        }
    })
}

async fn execute_data_tool(
    context: &AgentStreamContext,
    call: &AgentToolCall,
    provider_id: &ProviderId,
) -> Result<AgentToolOutput, magenta_core::ProviderError> {
    let output = execute_agent_data_tool(context, call).await;
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

async fn execute_workspace_tool(
    context: &AgentStreamContext,
    call: AgentToolCall,
    approvals: &Receiver<ApprovalResponse>,
    provider_id: &ProviderId,
    permissions: &AgentRunPermissionsHandle,
) -> Result<Vec<AgentRunEvent>, magenta_core::ProviderError> {
    let mut prepared = match prepare_tool(
        &context.store,
        &context.workspace,
        &context.root,
        &context.conversation,
        &context.assistant_message,
        context.run_id,
        &call,
    )
    .await
    {
        Ok(prepared) => prepared,
        Err(error) => {
            let output = record_failure(context, &call, &error, provider_id).await?;
            return Ok(vec![AgentRunEvent::ToolResult(output)]);
        }
    };

    apply_content_cache(context, &mut prepared).await;

    let mut events = Vec::new();
    let mut preview = prepared.preview;
    let proposed_change = workspace_change(&call, &preview, WorkspaceChangeState::Proposed);
    if let Some(change) = proposed_change.clone() {
        events.push(AgentRunEvent::WorkspaceChange(change));
    }

    if prepared.operation.is_mutating() || preview.protected {
        let approval = workspace_approval(&call, &prepared.operation, &preview);
        if let Some(approval_events) = request_workspace_approval(
            context,
            &call,
            &approval,
            proposed_change,
            approvals,
            provider_id,
            permissions,
        )
        .await?
        {
            events.extend(approval_events);
            return Ok(events);
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
                    let output = record_failure(context, &call, &detail, provider_id).await?;
                    events.push(AgentRunEvent::ToolResult(output));
                    return Ok(events);
                }
            };
        }
    }

    let (output, change) = finish_tool(context, &prepared.call, preview)
        .await
        .map_err(|error| agent_error(provider_id, &error))?;
    if let Some(change) = change {
        events.push(AgentRunEvent::WorkspaceChange(change));
    }
    events.push(AgentRunEvent::ToolResult(output));
    Ok(events)
}

async fn request_workspace_approval(
    context: &AgentStreamContext,
    call: &AgentToolCall,
    approval: &AgentApprovalRequest,
    proposed_change: Option<AgentWorkspaceChange>,
    approvals: &Receiver<ApprovalResponse>,
    provider_id: &ProviderId,
    permissions: &AgentRunPermissionsHandle,
) -> Result<Option<Vec<AgentRunEvent>>, magenta_core::ProviderError> {
    let granted_for_run = approval.can_approve_for_run
        && permissions
            .lock()
            .is_ok_and(|permissions| permissions.approve_workspace_edits);
    if granted_for_run {
        return Ok(None);
    }

    record_workspace_approval(context, call, approval, provider_id).await?;
    let decision = await_decision(approvals, &approval.request_id).await;
    if decision != AgentApprovalDecision::Reject {
        if decision == AgentApprovalDecision::ApproveWorkspaceEditsForRun
            && approval.can_approve_for_run
            && let Ok(mut permissions) = permissions.lock()
        {
            permissions.approve_workspace_edits = true;
        }
        return Ok(None);
    }

    let (change, output) =
        reject_workspace_tool(context, call, proposed_change, provider_id).await?;
    let mut events = Vec::with_capacity(2);
    if let Some(change) = change {
        events.push(AgentRunEvent::WorkspaceChange(change));
    }
    events.push(AgentRunEvent::ToolResult(output));
    Ok(Some(events))
}

async fn apply_content_cache(context: &AgentStreamContext, prepared: &mut PreparedTool) {
    if !matches!(prepared.operation, WorkspaceOperation::ReadFile { .. })
        || prepared.preview.protected
        || prepared.preview.output.is_empty()
    {
        return;
    }

    let Some(services) = &context.context_services else {
        return;
    };

    let content_hash = format!("{:x}", Sha256::digest(prepared.preview.output.as_bytes()));

    if let Ok(Some(cached)) = services
        .cache
        .cached_content(
            context.root.clone(),
            prepared.preview.path.clone(),
            content_hash.clone(),
        )
        .await
    {
        prepared.preview.output = cached.compact_context;
        prepared.preview.summary.push_str(" (content cache hit)");
        return;
    }

    let compact_context = compact_content(&prepared.preview.output);
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|value| i64::try_from(value.as_millis()).ok())
        .unwrap_or_default();

    let _ = services
        .cache
        .cache_content(
            context.root.clone(),
            magenta_core::CachedContent {
                path: prepared.preview.path.clone(),
                content_hash,
                version: timestamp,
                content: prepared.preview.output.clone(),
                compact_context,
                byte_size: u64::try_from(prepared.preview.output.len()).unwrap_or(u64::MAX),
                accessed_at: magenta_core::Timestamp(timestamp),
            },
        )
        .await;
}

fn compact_content(content: &str) -> String {
    const LIMIT: usize = 4_000;

    if content.len() <= LIMIT {
        return content.to_owned();
    }

    let mut end = LIMIT;
    while !content.is_char_boundary(end) {
        end -= 1;
    }

    format!(
        "{}\n… cached content truncated; use a narrower line range for full detail",
        &content[..end]
    )
}

async fn execute_agent_data_tool(
    context: &AgentStreamContext,
    call: &AgentToolCall,
) -> AgentToolOutput {
    let result = execute_agent_data_tool_result(context, call).await;

    match result {
        Ok(output) => AgentToolOutput {
            call_id: call.id.clone(),
            output,
            is_error: false,
        },
        Err(output) => AgentToolOutput {
            call_id: call.id.clone(),
            output,
            is_error: true,
        },
    }
}

async fn execute_agent_data_tool_result(
    context: &AgentStreamContext,
    call: &AgentToolCall,
) -> Result<String, String> {
    let services = context
        .context_services
        .as_ref()
        .ok_or_else(|| "agent data services are unavailable".to_owned())?;
    let arguments: serde_json::Value = serde_json::from_str(&call.arguments)
        .map_err(|error| format!("invalid tool arguments: {error}"))?;

    match call.name.as_str() {
        "search_code" => search_code(context, services, &arguments).await,
        "save_memory_candidate" => save_memory_candidate(context, services, &arguments).await,
        _ => Err("unknown agent data tool".to_owned()),
    }
}

async fn search_code(
    context: &AgentStreamContext,
    services: &AgentContextServices,
    arguments: &serde_json::Value,
) -> Result<String, String> {
    let query = arguments
        .get("query")
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "query must be a non-empty string".to_owned())?;
    let embedding = services
        .embeddings
        .embed(vec![query.to_owned()])
        .await
        .ok()
        .and_then(|mut values| values.pop());
    let matches = services
        .code
        .search_code(
            context.root.clone(),
            query.to_owned(),
            embedding,
            context
                .review
                .as_ref()
                .map(|review| review.session_id.clone()),
            12,
        )
        .await
        .map_err(|error| error.to_string())?;

    Ok(matches
        .into_iter()
        .map(|item| {
            format!(
                "{}:{}-{} ({:.3})\n{}",
                item.chunk.path,
                item.chunk.start_line,
                item.chunk.end_line,
                item.score,
                item.chunk.content
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n"))
}

async fn save_memory_candidate(
    context: &AgentStreamContext,
    services: &AgentContextServices,
    arguments: &serde_json::Value,
) -> Result<String, String> {
    let memory_text = arguments
        .get("content")
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "content must be a non-empty string".to_owned())?;
    let kind = match arguments.get("kind").and_then(serde_json::Value::as_str) {
        Some("fact") => MemoryKind::Fact,
        Some("preference") => MemoryKind::Preference,
        Some("decision") => MemoryKind::Decision,
        Some("procedure") => MemoryKind::Procedure,
        _ => return Err("unknown memory kind".to_owned()),
    };
    let confidence = arguments
        .get("confidence")
        .and_then(serde_json::Value::as_f64)
        .unwrap_or(0.5)
        .clamp(0.0, 1.0);
    let embedding = services
        .embeddings
        .embed(vec![memory_text.to_owned()])
        .await
        .ok()
        .and_then(|mut values| values.pop());
    let memory = services
        .memories
        .remember(
            context.root.clone(),
            NewAgentMemory {
                kind,
                state: MemoryState::Candidate,
                content: memory_text.to_owned(),
                source_conversation_id: Some(context.conversation.id),
                confidence,
                embedding,
            },
        )
        .await
        .map_err(|error| error.to_string())?;

    Ok(format!("saved memory candidate {} for review", memory.id))
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
            change.state = WorkspaceChangeState::Staged;
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
