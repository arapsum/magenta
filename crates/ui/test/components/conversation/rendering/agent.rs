use std::collections::HashMap;

use magenta_core::{
    ConversationId, WorkspaceCommand, WorkspaceCommandResult, WorkspaceCommandStatus,
};

use super::*;

fn activity(
    kind: AgentActivityKind,
    call_id: &str,
    tool_name: &str,
    status: &str,
) -> AgentActivity {
    let detail = if kind == AgentActivityKind::ToolCall {
        match tool_name {
            "run_command" => {
                r#"{"program":"cargo","args":[],"cwd":".","timeout_seconds":30}"#.to_owned()
            }
            _ => r#"{"path":"src/lib.rs"}"#.to_owned(),
        }
    } else {
        String::new()
    };
    AgentActivity {
        kind,
        call_id: call_id.to_owned(),
        tool_name: tool_name.to_owned(),
        status: status.to_owned(),
        summary: String::new(),
        detail,
    }
}

fn message(activities: Vec<AgentActivity>) -> Message {
    Message {
        id: MessageId::new(1),
        conversation_id: ConversationId::new(1),
        role: MessageRole::Assistant,
        content: String::new(),
        status: MessageStatus::Complete,
        attachments: Vec::new(),
        generation_outcome: None,
        failure: None,
        agent_activities: activities,
    }
}

#[test]
fn settled_sections_default_to_closed_and_new_activity_reopens_them() {
    let live_commands = HashMap::new();
    let mut message = message(vec![
        activity(
            AgentActivityKind::ToolCall,
            "tool-1",
            "read_file",
            "requested",
        ),
        activity(
            AgentActivityKind::ToolResult,
            "tool-1",
            "read_file",
            "completed",
        ),
        activity(
            AgentActivityKind::ToolCall,
            "command-1",
            "run_command",
            "requested",
        ),
        activity(
            AgentActivityKind::ToolResult,
            "command-1",
            "run_command",
            "completed",
        ),
    ]);
    assert!(!tool_call_section_is_active(&message));
    assert!(!command_section_is_active(&message, &live_commands));

    message.agent_activities.push(activity(
        AgentActivityKind::ToolCall,
        "tool-2",
        "list_files",
        "requested",
    ));
    assert!(tool_call_section_is_active(&message));
    assert!(!command_section_is_active(&message, &live_commands));
}

#[test]
fn commands_and_tool_calls_settle_independently() {
    let live_commands = HashMap::new();
    let message = message(vec![
        activity(
            AgentActivityKind::ToolCall,
            "tool-1",
            "read_file",
            "requested",
        ),
        activity(
            AgentActivityKind::ToolResult,
            "tool-1",
            "read_file",
            "failed",
        ),
        activity(
            AgentActivityKind::ToolCall,
            "command-1",
            "run_command",
            "requested",
        ),
    ]);
    assert!(!tool_call_section_is_active(&message));
    assert!(command_section_is_active(&message, &live_commands));
}

#[test]
fn cancelled_live_commands_are_settled() {
    let message = message(vec![activity(
        AgentActivityKind::ToolCall,
        "command-1",
        "run_command",
        "requested",
    )]);
    let mut live_commands = HashMap::new();
    live_commands.insert(
        (message.id, "command-1".to_owned()),
        LiveCommand {
            command: WorkspaceCommand {
                program: "cargo".to_owned(),
                args: Vec::new(),
                cwd: ".".to_owned(),
                timeout_seconds: 30,
            },
            stdout: String::new(),
            stderr: String::new(),
            result: Some(WorkspaceCommandResult {
                status: WorkspaceCommandStatus::Cancelled,
                exit_code: None,
                duration_ms: 0,
                stdout: String::new(),
                stderr: String::new(),
                truncated: false,
            }),
        },
    );
    assert!(!command_section_is_active(&message, &live_commands));
}
