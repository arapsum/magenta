use super::*;
use crate::app::runs::RunSaveState;
use crate::components::conversation::{GenerationProgress, LiveRunMessage};
use magenta_core::{
    AgentToolCall, Conversation, ConversationId, ConversationMode, EffortLevel, GenerationConfig,
    Message, MessageRole, ModelId, ProviderId, Timestamp,
};
use std::collections::HashMap;

fn coordinator() -> ResponseRunCoordinator {
    let generation = GenerationConfig::new(
        ProviderId::new("demo"),
        ModelId::new("demo"),
        EffortLevel::Medium,
    );
    let assistant = Message {
        id: MessageId(2),
        conversation_id: ConversationId(1),
        role: MessageRole::Assistant,
        command_id: None,
        content: String::new(),
        status: MessageStatus::Streaming,
        attachments: Vec::new(),
        generation_outcome: None,
        failure: None,
        assistant_trace: AssistantTrace::default(),
    };
    let run = ResponseRun {
        conversation: Conversation {
            id: ConversationId(1),
            title: "Search".to_owned(),
            generation: generation.clone(),
            mode: ConversationMode::Agent,
            workspace_root: None,
        },
        user: None,
        progress: GenerationProgress::new(
            assistant.id,
            generation.provider.clone(),
            Some(generation.clone()),
        ),
        assistant: LiveRunMessage {
            message: assistant,
            sequence: None,
            created_at: Timestamp(0),
            generation,
            omitted_context_messages: 0,
            replaces: None,
        },
        streaming: true,
        controller: None,
        pending_approval: None,
        live_commands: HashMap::new(),
        save_state: RunSaveState::Pending,
    };
    let mut coordinator = ResponseRunCoordinator::default();
    coordinator.runs.insert(MessageId(2), run);
    coordinator
}

#[test]
fn tool_results_preserve_the_request_and_record_successes_and_errors() {
    for (is_error, output, status) in [
        (
            false,
            "src/flavors.ts:12:peach",
            AssistantTraceStatus::Completed,
        ),
        (
            true,
            "Search failed: file is not UTF-8",
            AssistantTraceStatus::Failed,
        ),
    ] {
        let mut coordinator = coordinator();
        let arguments = r#"{"query":"peach","path":"src"}"#;
        coordinator.apply_agent_event(
            MessageId(2),
            AgentRunEvent::ToolCall(AgentToolCall {
                id: "search-1".to_owned(),
                name: "search_text".to_owned(),
                arguments: arguments.to_owned(),
            }),
        );
        coordinator.apply_agent_event(
            MessageId(2),
            AgentRunEvent::ToolResult(AgentToolOutput {
                call_id: "search-1".to_owned(),
                output: output.to_owned(),
                is_error,
            }),
        );
        let trace = &coordinator.runs[&MessageId(2)]
            .assistant
            .message
            .assistant_trace;
        assert_eq!(trace.entries.len(), 1);
        assert_eq!(trace.entries[0].input, arguments);
        assert_eq!(trace.entries[0].output, output);
        assert_eq!(trace.entries[0].status, status);
        assert!(trace.entries[0].finished_at.is_some());
    }
}

#[test]
fn final_command_result_replaces_partial_output_without_changing_arguments() {
    let mut coordinator = coordinator();
    let command = magenta_core::WorkspaceCommand {
        program: "cargo".to_owned(),
        args: vec!["test".to_owned()],
        cwd: ".".to_owned(),
        timeout_seconds: 60,
    };
    let arguments = serde_json::to_string(&command).unwrap();
    for event in [
        AgentRunEvent::ToolCall(AgentToolCall {
            id: "run-1".to_owned(),
            name: "run_command".to_owned(),
            arguments: arguments.clone(),
        }),
        AgentRunEvent::CommandStarted {
            call_id: "run-1".to_owned(),
            command,
        },
        AgentRunEvent::CommandOutput {
            call_id: "run-1".to_owned(),
            stream: WorkspaceCommandOutputStream::Stdout,
            chunk: "partial".to_owned(),
        },
        AgentRunEvent::ToolResult(AgentToolOutput {
            call_id: "run-1".to_owned(),
            is_error: false,
            output: serde_json::to_string(&WorkspaceCommandResult {
                status: magenta_core::WorkspaceCommandStatus::Exited,
                exit_code: Some(0),
                duration_ms: 120,
                stdout: "test passed".to_owned(),
                stderr: String::new(),
                truncated: false,
            })
            .unwrap(),
        }),
    ] {
        coordinator.apply_agent_event(MessageId(2), event);
    }
    let entry = &coordinator.runs[&MessageId(2)]
        .assistant
        .message
        .assistant_trace
        .entries[0];
    assert_eq!(entry.input, arguments);
    assert_eq!(entry.output, "test passed");
    assert_eq!(entry.status, AssistantTraceStatus::Completed);
}
