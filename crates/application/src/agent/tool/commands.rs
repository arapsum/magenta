use async_channel::Receiver;
use futures_util::StreamExt as _;
use magenta_core::{
    AgentApprovalDecision, AgentApprovalRequest, AgentApprovalSubject, AgentRunEvent,
    AgentRunStream, AgentToolCall, AgentToolOutput, WorkspaceCommand, WorkspaceCommandEvent,
    WorkspaceCommandResult, WorkspaceCommandStatus,
};

use super::super::{AgentStreamContext, ApprovalResponse};
use super::{await_decision, failed_output, rejected_output};

pub fn execute_command(
    context: AgentStreamContext,
    call: AgentToolCall,
    approvals: Receiver<ApprovalResponse>,
) -> AgentRunStream {
    Box::pin(async_stream::try_stream! {
        let command = match parse_command(&call)
            .and_then(|command| normalize_command_cwd(&context.root, command))
        {
            Ok(command) => command,
            Err(error) => {
                let output = failed_output(&call.id, &error);
                yield AgentRunEvent::ToolResult(output);
                return;
            }
        };

        let Some(runner) = context.command_runner.clone() else {
            let output = failed_output(&call.id, "sandboxed command execution is unavailable");
            yield AgentRunEvent::ToolResult(output);
            return;
        };
        let approval = command_approval(&call, &command);
        yield AgentRunEvent::ApprovalRequired(approval.clone());
        if await_decision(&approvals, &approval.request_id).await == AgentApprovalDecision::Reject {
            let output = rejected_output(&call.id, "the user rejected this command");
            yield AgentRunEvent::ToolResult(output);
            return;
        }

        yield AgentRunEvent::CommandStarted {
            call_id: call.id.clone(),
            command: command.clone(),
        };
        let mut events = runner.run(context.root.clone(), command);
        let mut result = None;
        while let Some(event) = events.next().await {
            match event {
                Ok(WorkspaceCommandEvent::Output { stream, chunk }) => {
                    yield AgentRunEvent::CommandOutput {
                        call_id: call.id.clone(),
                        stream,
                        chunk,
                    };
                }
                Ok(WorkspaceCommandEvent::Finished(finished)) => result = Some(finished),
                Err(error) => {
                    result = Some(WorkspaceCommandResult {
                        status: WorkspaceCommandStatus::Failed,
                        exit_code: None,
                        duration_ms: 0,
                        stdout: String::new(),
                        stderr: error.source.to_string(),
                        truncated: false,
                    });
                }
            }
        }
        let result = result.unwrap_or_else(|| WorkspaceCommandResult {
            status: WorkspaceCommandStatus::Failed,
            exit_code: None,
            duration_ms: 0,
            stdout: String::new(),
            stderr: "command stream ended without a result".to_owned(),
            truncated: false,
        });
        let is_error = result.status != WorkspaceCommandStatus::Exited
            || result.exit_code != Some(0);
        let output = AgentToolOutput {
            call_id: call.id.clone(),
            output: serde_json::to_string(&result).expect("command result serialization is infallible"),
            is_error,
        };
        yield AgentRunEvent::WorkspaceInvalidated;
        yield AgentRunEvent::ToolResult(output);
    })
}

fn command_approval(call: &AgentToolCall, command: &WorkspaceCommand) -> AgentApprovalRequest {
    AgentApprovalRequest {
        request_id: format!("{}-approval", call.id),
        tool_call_id: call.id.clone(),
        tool_name: call.name.clone(),
        reason: "This command can modify files in the selected workspace.".to_owned(),
        subject: AgentApprovalSubject::Command(command.clone()),
        can_approve_for_run: false,
    }
}

pub(super) fn parse_command(call: &AgentToolCall) -> Result<WorkspaceCommand, String> {
    let value: serde_json::Value = serde_json::from_str(&call.arguments)
        .map_err(|error| format!("invalid tool arguments: {error}"))?;
    let string = |name: &str| {
        value
            .get(name)
            .and_then(serde_json::Value::as_str)
            .map(ToOwned::to_owned)
            .ok_or_else(|| format!("tool argument {name} must be a string"))
    };
    let args = value
        .get("args")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| "tool argument args must be an array".to_owned())?
        .iter()
        .map(|argument| {
            argument
                .as_str()
                .map(ToOwned::to_owned)
                .ok_or_else(|| "command arguments must be strings".to_owned())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let timeout_seconds = value
        .get("timeout_seconds")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(120);
    let timeout_seconds =
        u16::try_from(timeout_seconds).map_err(|_| "command timeout is too large".to_owned())?;
    if !(1..=600).contains(&timeout_seconds) {
        return Err("command timeout must be between 1 and 600 seconds".to_owned());
    }
    Ok(WorkspaceCommand {
        program: string("program")?,
        args,
        cwd: string("cwd").map(|cwd| {
            if cwd.trim().is_empty() {
                ".".to_owned()
            } else {
                cwd
            }
        })?,
        timeout_seconds,
    })
}

pub(super) fn normalize_command_cwd(
    root: &std::path::Path,
    mut command: WorkspaceCommand,
) -> Result<WorkspaceCommand, String> {
    let cwd = std::path::Path::new(&command.cwd);
    if !cwd.is_absolute() {
        return Ok(command);
    }
    let root = root
        .canonicalize()
        .map_err(|error| format!("workspace root is unavailable: {error}"))?;
    let cwd = cwd
        .canonicalize()
        .map_err(|error| format!("command working directory is unavailable: {error}"))?;
    let relative = cwd
        .strip_prefix(&root)
        .map_err(|_| "command working directory is outside the selected workspace".to_owned())?;
    command.cwd = if relative.as_os_str().is_empty() {
        ".".to_owned()
    } else {
        relative.to_string_lossy().replace('\\', "/")
    };
    Ok(command)
}
