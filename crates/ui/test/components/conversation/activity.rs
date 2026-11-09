use super::*;
use futures_util::FutureExt as _;
use gpui_kit::Modifiers;

#[gpui_kit::test]
fn reduced_motion_refreshes_elapsed_time_until_the_final_answer(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        cx.set_reduce_motion(true);
    });
    let window = cx.open_window(size(px(900.), px(640.)), |window, cx| {
        let composer = cx.new(|cx| PromptComposer::new(window, cx));
        ConversationView::new(composer, window, cx)
    });
    let mut snapshot = LiveRunSnapshot {
        conversation: conversation(),
        messages: Vec::new(),
        streaming_message: Some(MessageId(2)),
        generation_progress: Some(GenerationProgress::new(
            MessageId(2),
            ProviderId::new("demo"),
            None,
        )),
        agent_controller: None,
        pending_agent_approval: None,
        live_commands: HashMap::new(),
    };
    window
        .update(cx, |view, _, cx| {
            view.load(
                ConversationThread {
                    conversation: conversation(),
                    messages: vec![assistant()],
                },
                cx,
            );
            view.apply_live_run(snapshot.clone(), cx);
        })
        .unwrap();
    cx.run_until_parked();
    let entity = window.root(cx).unwrap();
    let mut notifications = cx.notifications(&entity);
    cx.executor().advance_clock(GENERATION_CLOCK_INTERVAL);
    cx.run_until_parked();
    assert_eq!(
        notifications.next().now_or_never(),
        Some(Some(())),
        "the elapsed clock must refresh without animation frames or provider events"
    );

    snapshot
        .generation_progress
        .as_mut()
        .unwrap()
        .final_answer_started = true;
    window
        .update(cx, |view, _, cx| view.apply_live_run(snapshot, cx))
        .unwrap();
    cx.run_until_parked();
    while notifications.next().now_or_never().is_some() {}
    cx.executor().advance_clock(GENERATION_CLOCK_INTERVAL * 2);
    cx.run_until_parked();
    assert_eq!(
        notifications.next().now_or_never(),
        None,
        "the clock must stop refreshing once the answer starts"
    );
}

fn assistant() -> Message {
    let mut assistant = message(2, MessageRole::Assistant, MessageStatus::Streaming);
    assistant.content.clear();
    assistant.assistant_trace.entries = vec![AssistantTraceEntry {
        key: "reasoning:summary".to_owned(),
        sequence: 0,
        kind: AssistantTraceKind::ReasoningSummary,
        status: AssistantTraceStatus::Completed,
        title: "Thinking".to_owned(),
        tool_name: None,
        input: String::new(),
        output: "A provider-supplied summary.".to_owned(),
        started_at: None,
        finished_at: None,
    }];
    assistant
}

#[gpui_kit::test]
fn activity_tabs_and_manual_disclosure_survive_live_updates(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let window = cx.open_window(size(px(900.), px(640.)), |window, cx| {
        let composer = cx.new(|cx| PromptComposer::new(window, cx));
        ConversationView::new(composer, window, cx)
    });
    window
        .update(cx, |view, _, cx| {
            view.load(
                ConversationThread {
                    conversation: conversation(),
                    messages: vec![assistant()],
                },
                cx,
            );
        })
        .unwrap();
    let mut visual = gpui_kit::VisualTestContext::from_window(window.into(), cx);
    visual.run_until_parked();
    let reasoning = visual
        .debug_bounds("assistant-activity-tab-Reasoning")
        .unwrap();
    visual.simulate_click(reasoning.center(), Modifiers::default());
    visual.run_until_parked();
    assert!(
        visual
            .debug_bounds("assistant-activity-reasoning")
            .is_some()
    );
    let header = visual.debug_bounds("assistant-trace-header").unwrap();
    visual.simulate_click(
        gpui_kit::point(header.left() + px(40.), header.center().y),
        Modifiers::default(),
    );
    visual.run_until_parked();
    assert!(visual.debug_bounds("assistant-trace-viewport").is_none());
    window
        .update(cx, |view, _, cx| {
            let conversation = conversation();
            view.apply_live_run(
                LiveRunSnapshot {
                    conversation: conversation.clone(),
                    messages: vec![LiveRunMessage {
                        message: assistant(),
                        sequence: None,
                        created_at: Timestamp(0),
                        generation: conversation.generation,
                        omitted_context_messages: 0,
                        replaces: None,
                    }],
                    streaming_message: Some(MessageId(2)),
                    generation_progress: Some(GenerationProgress::new(
                        MessageId(2),
                        ProviderId::new("demo"),
                        None,
                    )),
                    agent_controller: None,
                    pending_agent_approval: None,
                    live_commands: HashMap::new(),
                },
                cx,
            );
            assert_eq!(
                view.trace_tab_selections.get(&MessageId(2)),
                Some(&TraceTab::Reasoning)
            );
            assert_eq!(
                view.trace_disclosure_overrides.get(&MessageId(2)),
                Some(&TraceDisclosureOverride::Closed)
            );
        })
        .unwrap();
    visual.run_until_parked();
    assert!(visual.debug_bounds("assistant-trace-viewport").is_none());
    assert!(visual.debug_bounds("assistant-grid-loader").is_some());
}

#[gpui_kit::test]
fn final_answer_collapses_activity_and_removes_loading_animation(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let window = cx.open_window(size(px(900.), px(640.)), |window, cx| {
        let composer = cx.new(|cx| PromptComposer::new(window, cx));
        ConversationView::new(composer, window, cx)
    });
    window
        .update(cx, |view, _, cx| {
            view.load(
                ConversationThread {
                    conversation: conversation(),
                    messages: vec![assistant()],
                },
                cx,
            );
            view.streaming_message = Some(MessageId(2));
            view.generation_progress = Some(GenerationProgress::new(
                MessageId(2),
                ProviderId::new("demo"),
                None,
            ));
        })
        .unwrap();
    let mut visual = gpui_kit::VisualTestContext::from_window(window.into(), cx);
    visual.run_until_parked();
    assert!(visual.debug_bounds("assistant-grid-loader").is_some());
    assert!(visual.debug_bounds("assistant-activity-stop").is_some());
    assert!(visual.debug_bounds("assistant-trace-viewport").is_some());
    window
        .update(cx, |view, _, cx| {
            view.generation_progress
                .as_mut()
                .unwrap()
                .final_answer_started = true;
            view.list_state.remeasure_items(0..1);
            cx.notify();
        })
        .unwrap();
    visual.run_until_parked();
    assert!(visual.debug_bounds("assistant-grid-loader").is_none());
    assert!(visual.debug_bounds("assistant-activity-stop").is_none());
    assert!(visual.debug_bounds("assistant-trace-viewport").is_none());
    assert!(visual.debug_bounds("assistant-trace-header").is_some());
    let header = visual.debug_bounds("assistant-trace-header").unwrap();
    visual.simulate_click(
        gpui_kit::point(header.left() + px(40.), header.center().y),
        Modifiers::default(),
    );
    visual.run_until_parked();
    assert!(visual.debug_bounds("assistant-trace-viewport").is_some());
}

#[gpui_kit::test]
fn more_search_results_expand_and_collapse_in_place(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let window = cx.open_window(size(px(900.), px(740.)), |window, cx| {
        let composer = cx.new(|cx| PromptComposer::new(window, cx));
        ConversationView::new(composer, window, cx)
    });
    window
        .update(cx, |view, _, cx| {
            let mut assistant = assistant();
            assistant.assistant_trace.entries = vec![AssistantTraceEntry {
                key: "tool:search".to_owned(),
                sequence: 0,
                kind: AssistantTraceKind::Tool,
                status: AssistantTraceStatus::Completed,
                title: "search_text".to_owned(),
                tool_name: Some("search_text".to_owned()),
                input: r#"{"query":"flavor","path":"src"}"#.to_owned(),
                output:
                    "src/a.ts:1:flavor\nsrc/b.ts:2:flavor\nsrc/c.ts:3:flavor\nsrc/d.ts:4:flavor"
                        .to_owned(),
                started_at: None,
                finished_at: None,
            }];
            view.load(
                ConversationThread {
                    conversation: conversation(),
                    messages: vec![assistant],
                },
                cx,
            );
            view.select_trace_tab(MessageId(2), TraceTab::Search, cx);
        })
        .unwrap();
    let mut visual = gpui_kit::VisualTestContext::from_window(window.into(), cx);
    visual.run_until_parked();
    assert!(
        visual
            .debug_bounds("assistant-activity-search-result-2")
            .is_some()
    );
    assert!(
        visual
            .debug_bounds("assistant-activity-search-result-3")
            .is_none()
    );
    let more = visual.debug_bounds("assistant-search-more").unwrap();
    visual.simulate_click(more.center(), Modifiers::default());
    visual.run_until_parked();
    assert!(
        visual
            .debug_bounds("assistant-activity-search-result-3")
            .is_some()
    );
    let less = visual.debug_bounds("assistant-search-more").unwrap();
    visual.simulate_click(less.center(), Modifiers::default());
    visual.run_until_parked();
    assert!(
        visual
            .debug_bounds("assistant-activity-search-result-3")
            .is_none()
    );
}
