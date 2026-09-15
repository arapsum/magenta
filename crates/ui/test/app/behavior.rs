use super::*;

#[gpui_kit::test]
fn new_chat_layout_keeps_primary_content_visible(cx: &mut TestAppContext) {
    let ports = Arc::new(TestPorts::default());
    ports
        .summaries
        .lock()
        .extend([summary(1, "A recent thread"), summary(2, "Another thread")]);
    let (window, _) = setup(cx, ports);
    cx.run_until_parked();

    let mut visual = gpui_kit::VisualTestContext::from_window(window.into(), cx);
    visual.run_until_parked();

    let start = visual
        .debug_bounds("new-chat-start-content")
        .expect("the new-chat start content should be rendered");
    let composer = visual
        .debug_bounds("prompt-composer-surface")
        .expect("the prompt composer should be rendered");
    let account = visual
        .debug_bounds("account-dropdown-trigger")
        .expect("the sidebar account control should be rendered");

    assert!(start.origin.x >= px(272.));
    assert!(composer.origin.y >= px(32.));
    assert!(composer.origin.y + composer.size.height <= px(700.));
    assert!(composer.size.width <= px(672.));
    assert!(account.origin.y + account.size.height <= px(700.));
}

#[gpui_kit::test]
fn narrow_new_chat_layout_keeps_the_composer_in_view(cx: &mut TestAppContext) {
    let (window, _) = setup_at(cx, Arc::new(TestPorts::default()), size(px(680.), px(640.)));
    cx.run_until_parked();

    let mut visual = gpui_kit::VisualTestContext::from_window(window.into(), cx);
    visual.run_until_parked();

    let start = visual
        .debug_bounds("new-chat-start-content")
        .expect("the narrow start content should be rendered");
    let composer = visual
        .debug_bounds("prompt-composer-surface")
        .expect("the narrow prompt composer should be rendered");

    assert!(start.origin.x >= px(16.));
    assert!(composer.origin.y >= px(32.));
    assert!(composer.origin.y + composer.size.height <= px(640.));
    assert!(composer.size.width <= px(616.));
    assert!(visual.debug_bounds("account-dropdown-trigger").is_none());
}

#[gpui_kit::test]
fn initialization_failure_can_be_retried_without_demo_history(cx: &mut TestAppContext) {
    let ports = Arc::new(TestPorts::default());
    ports.fail_initialize.store(true, Ordering::SeqCst);
    let (window, view) = setup(cx, ports.clone());
    cx.run_until_parked();
    view.read_with(cx, |main, _| {
        assert_eq!(main.storage_ready, StorageState::Failed);
        assert!(main.active_conversation.is_none());
    });
    ports.fail_initialize.store(false, Ordering::SeqCst);
    window
        .update(cx, |_, window, cx| {
            view.update(cx, |main, cx| main.load_history(window, cx));
        })
        .unwrap();
    cx.run_until_parked();
    assert!(view.read_with(cx, |main, _| main.storage_ready.is_ready()));
}

#[gpui_kit::test]
fn stale_and_failed_loads_keep_the_correct_selection(cx: &mut TestAppContext) {
    let ports = Arc::new(TestPorts::default());
    let (sender, receiver) = futures_channel::oneshot::channel();
    ports
        .loads
        .lock()
        .push_back(Box::pin(async move { receiver.await.unwrap() }));
    ports
        .loads
        .lock()
        .push_back(Box::pin(async { Ok(page(2)) }));
    let (window, view) = setup(cx, ports);
    cx.run_until_parked();
    window
        .update(cx, |_, window, cx| {
            view.update(cx, |main, cx| {
                main.navigate(Some(ConversationId(1)), window, cx);
            });
        })
        .unwrap();
    cx.run_until_parked();
    assert!(view.read_with(cx, |main, _| main.active_conversation.is_none()));
    window
        .update(cx, |_, window, cx| {
            view.update(cx, |main, cx| {
                main.navigate(Some(ConversationId(2)), window, cx);
            });
        })
        .unwrap();
    cx.run_until_parked();
    let _ = sender.send(Ok(page(1)));
    cx.run_until_parked();
    assert_eq!(
        view.read_with(cx, |main, _| main.active_conversation),
        Some(ConversationId(2))
    );
    window
        .update(cx, |_, window, cx| {
            view.update(cx, |main, cx| {
                main.navigate(Some(ConversationId(3)), window, cx);
            });
        })
        .unwrap();
    cx.run_until_parked();
    view.read_with(cx, |main, _| {
        assert_eq!(main.active_conversation, Some(ConversationId(2)));
        assert!(main.loading_conversation.is_none());
    });
}

#[gpui_kit::test]
fn switching_conversations_immediately_closes_and_clears_the_workbench(cx: &mut TestAppContext) {
    let ports = Arc::new(TestPorts::default());
    let (window, view) = setup_with_projects(cx, ports);
    cx.run_until_parked();

    window
        .update(cx, |_, window, cx| {
            view.update(cx, |main, cx| {
                let workbench = main.workbench.clone().unwrap();
                workbench.update(cx, |workbench, cx| {
                    workbench.show_change(
                        AgentWorkspaceChange {
                            call_id: "call-1".to_owned(),
                            path: "src/lib.rs".to_owned(),
                            kind: WorkspaceChangeKind::Modify,
                            content: "fn main() {}".to_owned(),
                            diff: String::new(),
                            state: WorkspaceChangeState::Committed,
                            error: None,
                        },
                        window,
                        cx,
                    );
                });
                main.active_conversation = Some(ConversationId(1));
                main.workbench_open = true;

                main.navigate(Some(ConversationId(2)), window, cx);

                assert!(!main.workbench_open);
                assert_eq!(workbench.read(cx).tab_count(), 0);
            });
        })
        .unwrap();
}

#[gpui_kit::test]
fn collapsed_sidebar_workbench_fills_a_wide_window(cx: &mut TestAppContext) {
    let ports = Arc::new(TestPorts::default());
    let (window, view) = setup_with_projects_at(cx, ports, size(px(2000.), px(800.)));
    cx.run_until_parked();

    window
        .update(cx, |_, window, cx| {
            view.update(cx, |main, cx| {
                main.sidebar.update(cx, |sidebar, cx| {
                    sidebar.toggle_collapsed(cx);
                    sidebar.set_active_project(Some(std::path::PathBuf::from("/workspace")), cx);
                });
                main.workbench.clone().unwrap().update(cx, |workbench, cx| {
                    workbench.show_change(
                        AgentWorkspaceChange {
                            call_id: "call-1".to_owned(),
                            path: "src/lib.rs".to_owned(),
                            kind: WorkspaceChangeKind::Modify,
                            content: "fn main() {}".to_owned(),
                            diff: String::new(),
                            state: WorkspaceChangeState::Committed,
                            error: None,
                        },
                        window,
                        cx,
                    );
                });
                main.workbench_open = true;
            });
        })
        .unwrap();

    let mut visual = gpui_kit::VisualTestContext::from_window(window.into(), cx);
    visual.run_until_parked();
    let workbench = visual
        .debug_bounds("agent-workbench")
        .expect("the code workbench should be rendered");
    let right_gutter = px(2000.) - (workbench.origin.x + workbench.size.width);
    assert!(
        right_gutter <= px(16.),
        "unexpected right gutter: {right_gutter:?}"
    );
}

#[gpui_kit::test]
fn failed_finalization_retains_response_until_retry_before_navigation(cx: &mut TestAppContext) {
    let ports = Arc::new(TestPorts::default());
    ports.fail_save.store(true, Ordering::SeqCst);
    let (window, view) = setup(cx, ports.clone());
    cx.run_until_parked();
    let response = Message {
        id: MessageId(7),
        conversation_id: ConversationId(1),
        role: MessageRole::Assistant,
        content: "A response to preserve".into(),
        status: MessageStatus::Complete,
        attachments: Vec::new(),
        generation_outcome: None,
        failure: None,
        assistant_trace: Default::default(),
    };
    window
        .update(cx, |_, window, cx| {
            view.update(cx, |main, cx| {
                main.active_conversation = Some(ConversationId(1));
                main.conversation
                    .update(cx, |conversation, cx| conversation.load_page(page(1), cx));
                main.save_response(response.clone(), window, cx);
            });
        })
        .unwrap();
    cx.run_until_parked();
    window
        .update(cx, |_, window, cx| {
            view.update(cx, |main, cx| {
                assert_eq!(main.unsaved.as_ref(), Some(&response));
                assert_eq!(main.operation, Operation::Idle);
                main.navigate(None, window, cx);
                assert_eq!(main.active_conversation, Some(ConversationId(1)));
                assert!(!main.request_close(window, cx));
            });
        })
        .unwrap();
    ports.fail_save.store(false, Ordering::SeqCst);
    window
        .update(cx, |_, window, cx| {
            view.update(cx, |main, cx| main.retry_save(window, cx));
        })
        .unwrap();
    cx.run_until_parked();
    view.read_with(cx, |main, _| {
        assert!(main.unsaved.is_none());
        assert!(main.active_conversation.is_none());
    });
    assert_eq!(ports.saves.lock().as_slice(), &[response.clone(), response]);
}

#[gpui_kit::test]
fn delete_confirmation_removes_the_thread_and_clears_the_active_view(cx: &mut TestAppContext) {
    let ports = Arc::new(TestPorts::default());
    ports.summaries.lock().push(summary(1, "Delete me"));
    let (window, view) = setup(cx, ports.clone());
    cx.run_until_parked();

    window
        .update(cx, |_, window, cx| {
            view.update(cx, |main, cx| {
                main.active_conversation = Some(ConversationId(1));
                main.conversation
                    .update(cx, |conversation, cx| conversation.load_page(page(1), cx));
                main.sidebar.update(cx, |sidebar, cx| {
                    sidebar.set_active(Some(ConversationId(1)), cx);
                });
                main.confirm_delete_conversation(ConversationId(1), window, cx);
            });
        })
        .unwrap();

    let mut visual = gpui_kit::VisualTestContext::from_window(window.into(), cx);
    visual.run_until_parked();
    assert!(view.read_with(cx, |main, _| main.pending_deletion.is_some()));

    let delete_bounds = visual
        .debug_bounds("confirm-delete-conversation")
        .expect("the delete confirmation button should be rendered");
    visual.simulate_click(delete_bounds.center(), gpui_kit::Modifiers::default());
    visual.run_until_parked();

    assert_eq!(ports.deleted.lock().as_slice(), &[ConversationId(1)]);
    view.read_with(cx, |main, _| {
        assert!(main.active_conversation.is_none());
        assert!(main.pending_deletion.is_none());
        assert_eq!(main.operation, Operation::Idle);
    });
}
