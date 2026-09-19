use super::*;

pub(super) fn execute_workspace_tool(
    context: AgentStreamContext,
    call: AgentToolCall,
    approvals: Receiver<ApprovalResponse>,
    provider_id: ProviderId,
    permissions: AgentRunPermissionsHandle,
) -> magenta_core::AgentRunStream {
    Box::pin(async_stream::try_stream! {
        let mut prepared = match prepare_tool(&context.workspace, &context.root, &call).await {
            Ok(prepared) => prepared,
            Err(error) => {
                let output = failed_output(&call.id, &error);
                yield AgentRunEvent::ToolResult(output);
                return;
            }
        };

        apply_content_cache(&context, &mut prepared).await;

        let mut preview = prepared.preview;
        let proposed_change = workspace_change(&call, &preview, WorkspaceChangeState::Proposed);
        if let Some(change) = proposed_change.clone() {
            yield AgentRunEvent::WorkspaceChange(change);
        }

        if prepared.operation.is_mutating() || preview.protected {
            let approval = workspace_approval(&call, &prepared.operation, &preview);
            if !workspace_approval_granted(&approval, &permissions) {
                yield AgentRunEvent::ApprovalRequired(approval.clone());
            }
            let approval_events = request_workspace_approval(
                &call,
                &approval,
                proposed_change,
                &approvals,
                &permissions,
            )
            .await?;
            if let Some(approval_events) = approval_events {
                for event in approval_events {
                    yield event;
                }
                return;
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
                        let output = failed_output(&call.id, &detail);
                        yield AgentRunEvent::ToolResult(output);
                        return;
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
    })
}

fn workspace_approval_granted(
    approval: &AgentApprovalRequest,
    permissions: &AgentRunPermissionsHandle,
) -> bool {
    approval.can_approve_for_run
        && permissions
            .lock()
            .is_ok_and(|permissions| permissions.approve_workspace_edits)
}

async fn request_workspace_approval(
    call: &AgentToolCall,
    approval: &AgentApprovalRequest,
    proposed_change: Option<AgentWorkspaceChange>,
    approvals: &Receiver<ApprovalResponse>,
    permissions: &AgentRunPermissionsHandle,
) -> Result<Option<Vec<AgentRunEvent>>, magenta_core::ProviderError> {
    if workspace_approval_granted(approval, permissions) {
        return Ok(None);
    }

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

    let (change, output) = reject_workspace_tool(call, proposed_change);
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

    let compact_context = super::data::compact_content(&prepared.preview.output);
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
struct PreparedTool {
    call: AgentToolCall,
    operation: WorkspaceOperation,
    preview: WorkspacePreview,
}

fn reject_workspace_tool(
    call: &AgentToolCall,
    proposed_change: Option<AgentWorkspaceChange>,
) -> (Option<AgentWorkspaceChange>, AgentToolOutput) {
    let change = proposed_change.map(|mut change| {
        change.state = WorkspaceChangeState::Rejected;
        change
    });

    let output = rejected_output(&call.id, "the user rejected this operation");

    (change, output)
}

async fn prepare_tool(
    workspace: &Arc<dyn WorkspaceAccess>,
    root: &Path,
    call: &AgentToolCall,
) -> Result<PreparedTool, String> {
    let operation = parse_operation(call)?;

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

pub(super) const fn can_approve_for_run(
    operation: &WorkspaceOperation,
    preview: &WorkspacePreview,
) -> bool {
    !preview.protected
        && matches!(
            operation,
            WorkspaceOperation::ApplyPatch { .. }
                | WorkspaceOperation::CreateFile { .. }
                | WorkspaceOperation::CreateDirectory { .. }
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
        kind: if mutation.creates_file || mutation.creates_directory {
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
