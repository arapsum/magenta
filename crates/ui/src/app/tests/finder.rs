use super::*;

#[gpui::test]
fn finder_filters_persisted_history_and_opens_selected_conversation(cx: &mut TestAppContext) {
    let ports = Arc::new(TestPorts::default());
    ports.summaries.lock().extend([
        summary(1, "Design notes"),
        summary(2, "Streaming responses"),
    ]);
    ports
        .loads
        .lock()
        .push_back(Box::pin(async { Ok(page(2)) }));

    let (window, view) = setup(cx, ports);
    cx.run_until_parked();

    let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
    visual.dispatch_action(OpenConversationFinder);
    visual.run_until_parked();
    assert!(view.read_with(cx, |main, _| main.finder_open.is_open()));
    visual.simulate_input("responses");
    visual.run_until_parked();

    let result_bounds = visual
        .debug_bounds("finder-conversation-2")
        .expect("the filtered conversation should be rendered");
    visual.simulate_click(result_bounds.center(), gpui::Modifiers::default());
    visual.run_until_parked();

    view.read_with(cx, |main, _| {
        assert_eq!(main.finder_open, PanelState::Closed);
        assert_eq!(main.active_conversation, Some(ConversationId(2)));
    });
    let sidebar = view.read_with(cx, |main, _| main.sidebar.clone());
    assert_eq!(
        sidebar.read_with(cx, |sidebar, _| sidebar.active_conversation()),
        Some(ConversationId(2))
    );
}

#[gpui::test]
fn finder_message_match_loads_the_page_around_that_message(cx: &mut TestAppContext) {
    let ports = Arc::new(TestPorts::default());
    ports
        .summaries
        .lock()
        .push(summary(7, "Architecture notes"));
    *ports.search_results.lock() = Some(vec![ConversationSearchResult {
        conversation_id: ConversationId(7),
        message_id: Some(MessageId(70)),
        message_sequence: Some(MessageSequence(64)),
        title: "Architecture notes".into(),
        title_highlights: Vec::new(),
        snippet: "The indexing needle is here".into(),
        snippet_highlights: std::iter::once(13..19).collect(),
        updated_at: Timestamp(0),
    }]);
    ports
        .loads
        .lock()
        .push_back(Box::pin(async { Ok(page(7)) }));

    let (window, view) = setup(cx, ports.clone());
    cx.run_until_parked();
    let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
    visual.dispatch_action(OpenConversationFinder);
    visual.simulate_input("needle");
    visual.run_until_parked();
    let result = visual
        .debug_bounds("finder-conversation-7")
        .expect("the message body match should be rendered");
    visual.simulate_click(result.center(), gpui::Modifiers::default());
    visual.run_until_parked();

    assert_eq!(
        ports.around_loads.lock().as_slice(),
        &[(ConversationId(7), MessageSequence(64))]
    );
    assert_eq!(
        view.read_with(cx, |main, _| main.active_conversation),
        Some(ConversationId(7))
    );
}

#[gpui::test]
fn closing_finder_clears_query_without_changing_selection(cx: &mut TestAppContext) {
    let ports = Arc::new(TestPorts::default());
    ports.summaries.lock().push(summary(1, "Design notes"));

    let (window, view) = setup(cx, ports);
    cx.run_until_parked();

    let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
    visual.dispatch_action(OpenConversationFinder);
    visual.run_until_parked();
    assert!(view.read_with(cx, |main, _| main.finder_open.is_open()));
    visual.simulate_input("notes");
    visual.run_until_parked();
    visual.simulate_keystrokes("escape");
    visual.run_until_parked();

    view.read_with(cx, |main, app| {
        assert_eq!(main.finder_open, PanelState::Closed);
        assert!(main.finder_input.read(app).value().is_empty());
        assert!(main.active_conversation.is_none());
    });
}

#[gpui::test]
fn finder_arrow_navigation_opens_highlighted_conversation(cx: &mut TestAppContext) {
    let ports = Arc::new(TestPorts::default());
    ports.summaries.lock().extend([
        summary(1, "Design notes"),
        summary(2, "Streaming responses"),
    ]);
    ports
        .loads
        .lock()
        .push_back(Box::pin(async { Ok(page(2)) }));

    let (window, view) = setup(cx, ports);
    cx.run_until_parked();

    let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
    visual.dispatch_action(OpenConversationFinder);
    visual.run_until_parked();
    visual.simulate_keystrokes("down enter");
    visual.run_until_parked();

    view.read_with(cx, |main, _| {
        assert_eq!(main.finder_open, PanelState::Closed);
        assert_eq!(main.active_conversation, Some(ConversationId(2)));
    });
}

#[gpui::test]
fn finder_result_list_scrolls_when_history_exceeds_the_dialog(cx: &mut TestAppContext) {
    let ports = Arc::new(TestPorts::default());
    ports
        .summaries
        .lock()
        .extend((1..=20).map(|id| summary(id, &format!("Conversation {id}"))));
    let (window, _) = setup(cx, ports);
    cx.run_until_parked();

    let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
    visual.dispatch_action(OpenConversationFinder);
    visual.run_until_parked();
    let list = visual
        .debug_bounds("finder-result-list")
        .expect("the finder result viewport should be rendered");
    let first_before = visual
        .debug_bounds("finder-conversation-20")
        .expect("the first conversation should be rendered")
        .top();

    visual.simulate_event(gpui::ScrollWheelEvent {
        position: gpui::point(list.center().x, list.top() + px(150.)),
        delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-240.))),
        ..Default::default()
    });
    visual.run_until_parked();

    let first_after = visual
        .debug_bounds("finder-conversation-20")
        .expect("the first conversation should remain in the scroll content")
        .top();
    assert!(
        first_after < first_before,
        "expected scroll offset to change: before={first_before:?}, after={first_after:?}, list={list:?}"
    );
}
