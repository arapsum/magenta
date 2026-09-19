use magenta_core::{
    AssistantTrace, AssistantTraceEntry, AssistantTraceKind, AssistantTraceStatus, ConversationId,
};

use super::*;

fn trace_entry(
    key: &str,
    sequence: u64,
    kind: AssistantTraceKind,
    status: AssistantTraceStatus,
    title: &str,
    tool_name: Option<&str>,
) -> AssistantTraceEntry {
    AssistantTraceEntry {
        key: key.to_owned(),
        sequence,
        kind,
        status,
        title: title.to_owned(),
        tool_name: tool_name.map(str::to_owned),
        input: String::new(),
        output: String::new(),
        started_at: None,
        finished_at: None,
    }
}

fn message(trace: AssistantTrace) -> Message {
    Message {
        id: MessageId::new(1),
        conversation_id: ConversationId::new(1),
        role: MessageRole::Assistant,
        content: String::new(),
        status: MessageStatus::Complete,
        attachments: Vec::new(),
        generation_outcome: None,
        failure: None,
        assistant_trace: trace,
    }
}

#[test]
fn trace_entries_render_newest_first_for_reverse_timeline() {
    let trace = AssistantTrace {
        entries: vec![
            trace_entry(
                "tool:command-1",
                1,
                AssistantTraceKind::Tool,
                AssistantTraceStatus::Completed,
                "run_command",
                Some("run_command"),
            ),
            trace_entry(
                "reasoning:item-1:0",
                0,
                AssistantTraceKind::ReasoningSummary,
                AssistantTraceStatus::Completed,
                "Thinking",
                None,
            ),
        ],
        thinking_duration_ms: Some(1200),
    };
    let message = message(trace);
    let entries = trace_entries_for_timeline(&message);
    assert_eq!(entries[0].key, "tool:command-1");
    assert_eq!(entries[1].key, "reasoning:item-1:0");
}

#[test]
fn reasoning_rows_use_a_distinct_summary_label() {
    let entry = trace_entry(
        "reasoning:item-1:0",
        0,
        AssistantTraceKind::ReasoningSummary,
        AssistantTraceStatus::Completed,
        "Thinking",
        None,
    );

    assert_eq!(trace_entry_title(&entry), "Reasoning");
}

#[test]
fn active_trace_entries_keep_the_timeline_open_by_default() {
    let message = Message {
        status: MessageStatus::Streaming,
        assistant_trace: AssistantTrace {
            entries: vec![trace_entry(
                "tool:command-1",
                0,
                AssistantTraceKind::Tool,
                AssistantTraceStatus::Running,
                "run_command",
                Some("run_command"),
            )],
            thinking_duration_ms: None,
        },
        ..message(AssistantTrace::default())
    };
    assert_eq!(message.status, MessageStatus::Streaming);
    assert!(
        message
            .assistant_trace
            .entries
            .iter()
            .any(|entry| matches!(entry.status, AssistantTraceStatus::Running))
    );
}
