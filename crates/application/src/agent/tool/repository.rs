use async_channel::Receiver;
use magenta_core::{
    AgentApprovalDecision, AgentApprovalRequest, AgentApprovalSubject, AgentRunEvent,
    AgentRunStream, AgentToolCall, AgentToolOutput, RepositoryAccess, RepositoryChangeKind,
    RepositoryDiffArea, WorkspaceOperation,
};

use super::{AgentStreamContext, ApprovalResponse, await_decision, failed_output, rejected_output};

pub(super) fn execute_repository_tool(
    context: AgentStreamContext,
    call: AgentToolCall,
    approvals: Receiver<ApprovalResponse>,
) -> AgentRunStream {
    Box::pin(async_stream::try_stream! {
        if call.name == "repository_diff"
            && let Some(path) = diff_path(&call)
            && is_protected_path(&context, &path).await
        {
            let approval = AgentApprovalRequest {
                request_id: format!("{}-approval", call.id),
                tool_call_id: call.id.clone(),
                tool_name: call.name.clone(),
                reason: "This repository path may contain credentials or secrets.".to_owned(),
                subject: AgentApprovalSubject::Workspace {
                    path,
                    diff: None,
                    protected_read: true,
                },
                can_approve_for_run: false,
            };
            yield AgentRunEvent::ApprovalRequired(approval.clone());
            if await_decision(&approvals, &approval.request_id).await == AgentApprovalDecision::Reject {
                yield AgentRunEvent::ToolResult(rejected_output(
                    &call.id,
                    "the user rejected this operation",
                ));
                return;
            }
        }

        yield AgentRunEvent::ToolResult(execute_repository_tool_once(&context, &call).await);
    })
}

async fn execute_repository_tool_once(
    context: &AgentStreamContext,
    call: &AgentToolCall,
) -> AgentToolOutput {
    let Some(repository) = context.repository.as_ref() else {
        return failed_output(&call.id, "repository access is unavailable");
    };

    let result = match call.name.as_str() {
        "repository_status" => repository
            .status(context.root.clone())
            .await
            .map_err(|error| error.to_string())
            .map(|status| {
                serde_json::json!({
                    "branch": status.branch,
                    "detached": status.detached,
                    "unborn": status.unborn,
                    "changes": status.changes.into_iter().map(|change| serde_json::json!({
                        "path": change.path,
                        "original_path": change.original_path,
                        "staged": change.staged.map(change_kind),
                        "unstaged": change.unstaged.map(change_kind),
                    })).collect::<Vec<_>>(),
                })
            }),
        "repository_diff" => diff(context, call, repository.as_ref()).await,
        _ => Err("unknown repository tool".to_owned()),
    };

    match result {
        Ok(value) => AgentToolOutput {
            call_id: call.id.clone(),
            output: value.to_string(),
            is_error: false,
        },
        Err(error) => failed_output(&call.id, &error),
    }
}

fn diff_path(call: &AgentToolCall) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(&call.arguments)
        .ok()?
        .get("path")
        .and_then(serde_json::Value::as_str)
        .filter(|path| !path.trim().is_empty())
        .map(ToOwned::to_owned)
}

async fn is_protected_path(context: &AgentStreamContext, path: &str) -> bool {
    context
        .workspace
        .prepare(
            context.root.clone(),
            WorkspaceOperation::ReadFile {
                path: path.to_owned(),
                start_line: None,
                line_count: None,
            },
            false,
        )
        .await
        .is_ok_and(|preview| preview.protected)
}

async fn diff(
    context: &AgentStreamContext,
    call: &AgentToolCall,
    repository: &dyn RepositoryAccess,
) -> Result<serde_json::Value, String> {
    let value: serde_json::Value = serde_json::from_str(&call.arguments)
        .map_err(|error| format!("invalid repository diff arguments: {error}"))?;
    let path = value
        .get("path")
        .and_then(serde_json::Value::as_str)
        .filter(|path| !path.trim().is_empty())
        .ok_or_else(|| "repository diff path must be a non-empty string".to_owned())?;
    let area = match value.get("area").and_then(serde_json::Value::as_str) {
        Some("staged") => RepositoryDiffArea::Staged,
        Some("unstaged") => RepositoryDiffArea::Unstaged,
        _ => return Err("repository diff area must be staged or unstaged".to_owned()),
    };
    let diff = repository
        .diff(context.root.clone(), path.to_owned(), area)
        .await
        .map_err(|error| error.to_string())?;
    Ok(serde_json::json!({
        "path": diff.path,
        "area": match diff.area {
            RepositoryDiffArea::Staged => "staged",
            RepositoryDiffArea::Unstaged => "unstaged",
        },
        "unified_diff": diff.unified_diff,
        "binary": diff.binary,
    }))
}

const fn change_kind(kind: RepositoryChangeKind) -> &'static str {
    match kind {
        RepositoryChangeKind::Added => "added",
        RepositoryChangeKind::Modified => "modified",
        RepositoryChangeKind::Deleted => "deleted",
        RepositoryChangeKind::Renamed => "renamed",
        RepositoryChangeKind::Untracked => "untracked",
        RepositoryChangeKind::Conflicted => "conflicted",
    }
}
