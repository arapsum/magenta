use magenta_core::{AssistantTrace, ConversationId};

use super::*;

fn tool(name: &str, input: &serde_json::Value, output: &str) -> AssistantTraceEntry {
    AssistantTraceEntry {
        key: format!("tool:{name}"),
        sequence: 0,
        kind: AssistantTraceKind::Tool,
        status: AssistantTraceStatus::Completed,
        title: name.to_owned(),
        tool_name: Some(name.to_owned()),
        input: input.to_string(),
        output: output.to_owned(),
        started_at: None,
        finished_at: None,
    }
}

fn message(entries: Vec<AssistantTraceEntry>) -> Message {
    Message {
        id: MessageId(1),
        conversation_id: ConversationId(1),
        role: MessageRole::Assistant,
        command_id: None,
        content: String::new(),
        status: MessageStatus::Complete,
        attachments: Vec::new(),
        generation_outcome: None,
        failure: None,
        assistant_trace: AssistantTrace {
            entries,
            thinking_duration_ms: None,
        },
    }
}

#[test]
fn tabs_partition_observed_activity_and_preserve_sequence() {
    let search = tool(
        "search_text",
        &serde_json::json!({"query": "flavor", "path": "."}),
        "",
    );
    let mut read = tool("read_file", &serde_json::json!({"path": "flavors.ts"}), "");
    read.sequence = 2;
    let mut reasoning = tool("unused", &serde_json::Value::Null, "A provider summary.");
    reasoning.kind = AssistantTraceKind::ReasoningSummary;
    reasoning.tool_name = None;
    reasoning.sequence = 1;
    let message = message(vec![read, reasoning, search]);
    assert_eq!(
        data::entries(&message, TraceTab::Steps)
            .iter()
            .map(|entry| entry.sequence)
            .collect::<Vec<_>>(),
        [0, 1, 2]
    );
    assert_eq!(
        data::entries(&message, TraceTab::Reasoning)[0].output,
        "A provider summary."
    );
    assert_eq!(data::entries(&message, TraceTab::Search).len(), 1);
    let tools = data::entries(&message, TraceTab::Tools);
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].tool_name.as_deref(), Some("read_file"));
}

#[test]
fn tools_display_real_arguments_and_tolerate_incomplete_json() {
    let mut command = tool(
        "run_command",
        &serde_json::json!({"program": "cargo", "args": ["test", "--workspace"], "cwd": "."}),
        "",
    );
    assert_eq!(
        data::target(&command).as_deref(),
        Some("cargo test --workspace")
    );
    command.input = "{\"program\":".to_owned();
    assert_eq!(data::target(&command), None);
}

#[test]
fn edit_counts_ignore_diff_headers_and_preserve_content_prefixes() {
    let mut edit = tool(
        "apply_patch",
        &serde_json::json!({
            "path": "schedule.tsx",
            "unified_diff": "--- a/schedule.tsx\n+++ b/schedule.tsx\n@@ -1,2 +1,3 @@\n-old\n+new\n+++content\n unchanged\n"
        }),
        "Applied",
    );
    assert_eq!(
        data::diff_stats(&edit),
        Some(data::DiffStats {
            added: 2,
            removed: 1
        })
    );
    edit.status = AssistantTraceStatus::AwaitingApproval;
    assert_eq!(data::diff_stats(&edit), None);
    edit.status = AssistantTraceStatus::Failed;
    assert_eq!(data::diff_stats(&edit), None);
}

#[test]
fn text_search_uses_real_file_matches_without_treating_errors_as_sources() {
    let mut search = tool(
        "search_text",
        &serde_json::Value::Null,
        "src/flavors.ts:12:peach: true\nsrc/a:b.ts:7:apricot\nnot a search result",
    );
    let matches = data::search_matches(&search);
    assert_eq!(matches.len(), 2);
    assert_eq!(matches[0].path, "src/flavors.ts");
    assert_eq!(matches[0].location, "12");
    assert_eq!(matches[0].excerpt, "peach: true");
    assert_eq!(matches[1].path, "src/a:b.ts");
    search.status = AssistantTraceStatus::Failed;
    assert_eq!(data::search_matches(&search), []);
}

#[test]
fn code_search_displays_recorded_ranges_and_excerpts() {
    let search = tool(
        "search_code",
        &serde_json::Value::Null,
        "src/flavors.ts:5-18 (0.934)\nexport const peach = true;\n\nsrc/schedule.tsx:20-38 (0.910)\nfunction schedule() {}",
    );
    let matches = data::search_matches(&search);
    assert_eq!(matches.len(), 2);
    assert_eq!(matches[0].location, "5-18");
    assert_eq!(matches[1].path, "src/schedule.tsx");
    assert_eq!(matches[1].excerpt, "function schedule() {}");
}

#[test]
fn loader_caption_tracks_search_and_pending_approval() {
    let mut message = message(vec![tool("search_text", &serde_json::Value::Null, "")]);
    message.assistant_trace.entries[0].status = AssistantTraceStatus::Running;
    assert_eq!(data::active_label(&message, None), "Searching");
    message.assistant_trace.entries[0].status = AssistantTraceStatus::AwaitingApproval;
    assert_eq!(data::active_label(&message, None), "Waiting for approval");
}

#[test]
fn loader_timer_keeps_tenths_across_minute_boundaries() {
    assert_eq!(loader::format_timer(Duration::ZERO), "0.0s");
    assert_eq!(loader::format_timer(Duration::from_millis(59_999)), "59.9s");
    assert_eq!(
        loader::format_timer(Duration::from_millis(60_000)),
        "1m 0.0s"
    );
    assert_eq!(
        loader::format_timer(Duration::from_millis(304_550)),
        "5m 4.5s"
    );
    assert_eq!(loader::format_timer(Duration::from_secs(3_725)), "1h 02m");
}
