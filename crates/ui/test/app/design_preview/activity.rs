use super::*;
use crate::components::conversation::{
    ConversationThread, GenerationPhase, GenerationProgress, LiveRunSnapshot, TraceTab,
};
use magenta_core::{AssistantTraceEntry, AssistantTraceKind, AssistantTraceStatus};
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

fn entry(
    name: &str,
    sequence: u64,
    input: &serde_json::Value,
    output: &str,
) -> AssistantTraceEntry {
    AssistantTraceEntry {
        key: format!("tool:{sequence}"),
        sequence,
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

fn trace_entries() -> Vec<AssistantTraceEntry> {
    let mut entries = vec![
        entry(
            "read_file",
            1,
            &serde_json::json!({"path":"src/flavors.ts"}),
            "export const seasonal = ['peach', 'apricot'];",
        ),
        entry(
            "search_text",
            2,
            &serde_json::json!({"query":"seasonal", "path":"src"}),
            "src/flavors.ts:12:export const seasonal = ['peach', 'apricot'];\nsrc/menu.ts:8:import { seasonal } from './flavors';\nsrc/suppliers.ts:24:const seasonalStock = await inventory();\nsrc/menu.test.ts:9:expect(seasonal).toContain('peach');",
        ),
        entry(
            "apply_patch",
            3,
            &serde_json::json!({"path":"src/SummerMenu.tsx", "unified_diff":"--- a/src/SummerMenu.tsx\n+++ b/src/SummerMenu.tsx\n@@ -1 +1,3 @@\n-const flavors = winter;\n+const flavors = seasonal;\n+const featured = 'peach';\n+const next = 'apricot';"}),
            "Updated src/SummerMenu.tsx",
        ),
        entry(
            "run_command",
            4,
            &serde_json::json!({"program":"npm", "args":["run", "check"], "cwd":".", "timeout_seconds":120}),
            "Checking types…",
        ),
    ];
    entries.insert(0, AssistantTraceEntry {
        key: "reasoning:summary".to_owned(), sequence: 0, kind: AssistantTraceKind::ReasoningSummary,
        status: AssistantTraceStatus::Completed, title: "Reviewing seasonal demand".to_owned(), tool_name: None,
        input: String::new(), output: "Summer demand is strongest for stone-fruit flavors — peach and apricot lead.\n\nThe menu should reflect what is in stock, so I’m checking the supplier list before adding the seasonal specials.".to_owned(),
        started_at: None, finished_at: None,
    });
    entries.last_mut().unwrap().status = AssistantTraceStatus::Running;
    entries
}

fn populate(cx: &mut HeadlessAppContext, view: &Entity<MainView>, loading_only: bool) {
    let generation = GenerationConfig::new(
        ProviderId::new("openai"),
        ModelId::new("gpt-6.1-terra"),
        EffortLevel::Medium,
    );
    let conversation = Conversation {
        id: ConversationId(1),
        title: "A little more summer".to_owned(),
        generation: generation.clone(),
        mode: ConversationMode::Chat,
        workspace_root: None,
    };
    let user = Message {
        id: MessageId(1),
        conversation_id: conversation.id,
        role: MessageRole::User,
        command_id: None,
        content: "Review our seasonal flavors and prepare the summer menu.".to_owned(),
        status: MessageStatus::Complete,
        attachments: Vec::new(),
        generation_outcome: None,
        failure: None,
        assistant_trace: AssistantTrace::default(),
    };
    let assistant = Message {
        id: MessageId(2),
        conversation_id: conversation.id,
        role: MessageRole::Assistant,
        command_id: None,
        content: String::new(),
        status: MessageStatus::Streaming,
        attachments: Vec::new(),
        generation_outcome: None,
        failure: None,
        assistant_trace: AssistantTrace {
            entries: if loading_only {
                Vec::new()
            } else {
                trace_entries()
            },
            thinking_duration_ms: None,
        },
    };
    let mut progress = GenerationProgress::new(assistant.id, generation.provider, None);
    progress.started_at = Instant::now()
        .checked_sub(Duration::from_millis(304_500))
        .unwrap();
    progress.phase = GenerationPhase::Thinking;
    view.update(cx, |main, cx| {
        main.active_conversation = Some(conversation.id);
        main.sidebar.update(cx, |sidebar, cx| {
            sidebar.set_active(main.active_conversation, cx);
        });
        main.conversation.update(cx, |view, cx| {
            view.load(
                ConversationThread {
                    conversation: conversation.clone(),
                    messages: vec![user, assistant],
                },
                cx,
            );
            view.apply_live_run(
                LiveRunSnapshot {
                    conversation,
                    messages: Vec::new(),
                    streaming_message: Some(MessageId(2)),
                    generation_progress: Some(progress),
                    agent_controller: None,
                    pending_agent_approval: None,
                    live_commands: HashMap::new(),
                },
                cx,
            );
        });
        cx.notify();
    });
}

#[test]
#[ignore = "Exports activity PNG previews using the native text system and a GPU renderer"]
fn export_activity_previews() {
    for (appearance, name, viewport) in [
        (AppearanceMode::Dark, "dark", (1180., 800.)),
        (AppearanceMode::Light, "light", (1180., 800.)),
        (AppearanceMode::Dark, "compact", (680., 640.)),
    ] {
        let (mut cx, handle, view) = preview(appearance, viewport, false);
        populate(&mut cx, &view, true);
        save(&mut cx, handle, &format!("{name}-loading"));
        populate(&mut cx, &view, false);
        for (tab, label) in [
            (TraceTab::Steps, "steps"),
            (TraceTab::Reasoning, "reasoning"),
            (TraceTab::Search, "search"),
            (TraceTab::Tools, "tools"),
        ] {
            view.update(&mut cx, |main, cx| {
                main.conversation
                    .update(cx, |view, cx| view.select_trace_tab(MessageId(2), tab, cx));
            });
            save(&mut cx, handle, &format!("{name}-activity-{label}"));
        }
    }
}
