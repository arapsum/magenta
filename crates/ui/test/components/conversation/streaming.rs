use magenta_core::{FinishReason, TokenUsage};

use super::*;

#[gpui_kit::test]
fn completed_stream_stores_its_outcome(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let window = cx.open_window(size(px(900.), px(640.)), |window, cx| {
        let composer = cx.new(|cx| PromptComposer::new(window, cx));
        ConversationView::new(composer, window, cx)
    });
    let outcome = GenerationOutcome::new(
        FinishReason::Length,
        Some(TokenUsage {
            input_tokens: 12,
            output_tokens: 24,
        }),
    );

    window
        .update(cx, |view, window, cx| {
            view.load(
                ConversationThread {
                    conversation: conversation(),
                    messages: Vec::new(),
                },
                cx,
            );
            view.start_generation(
                message(1, MessageRole::User, MessageStatus::Complete),
                message(2, MessageRole::Assistant, MessageStatus::Streaming),
                ProviderId::new("demo"),
                Box::pin(stream::iter([
                    Ok(GenerationEvent::Started),
                    Ok(GenerationEvent::TextDelta("hello λ".to_owned())),
                    Ok(GenerationEvent::Completed(outcome.clone())),
                ])),
                window,
                cx,
            );
        })
        .expect("the conversation test window should remain open");
    cx.run_until_parked();

    window
        .update(cx, |view, _, _| {
            let thread = view
                .snapshot()
                .expect("the conversation should remain loaded");
            let assistant = thread
                .messages
                .last()
                .expect("an assistant response should exist");
            assert_eq!(assistant.content, "hello λ");
            assert_eq!(assistant.status, MessageStatus::Complete);
            assert_eq!(assistant.generation_outcome, Some(outcome));
        })
        .expect("the conversation test window should remain open");
}

#[gpui_kit::test]
fn stream_ending_without_completion_marks_the_response_failed(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let view_slot = Rc::new(RefCell::new(None));
    let view_for_window = Rc::clone(&view_slot);
    let window = cx.open_window(size(px(900.), px(640.)), move |window, cx| {
        let composer = cx.new(|cx| PromptComposer::new(window, cx));
        let view = cx.new(|cx| ConversationView::new(composer, window, cx));
        view_for_window.replace(Some(view.clone()));
        Root::new(view, window, cx)
    });
    let view = view_slot
        .borrow()
        .clone()
        .expect("the conversation view should be created");

    window
        .update(cx, |_, window, cx| {
            view.update(cx, |view, cx| {
                view.load(
                    ConversationThread {
                        conversation: conversation(),
                        messages: Vec::new(),
                    },
                    cx,
                );
                view.start_generation(
                    message(1, MessageRole::User, MessageStatus::Complete),
                    message(2, MessageRole::Assistant, MessageStatus::Streaming),
                    ProviderId::new("demo"),
                    Box::pin(stream::iter([Ok(GenerationEvent::Started)])),
                    window,
                    cx,
                );
            });
        })
        .expect("the conversation test window should remain open");
    cx.run_until_parked();

    view.read_with(cx, |view, _| {
        let thread = view
            .snapshot()
            .expect("the conversation should remain loaded");
        assert_eq!(
            thread.messages.last().map(|message| message.status),
            Some(MessageStatus::Failed)
        );
    });
}

#[gpui_kit::test]
fn cancellation_drops_the_stream_and_rejects_stale_chunks(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let window = cx.open_window(size(px(900.), px(640.)), |window, cx| {
        let composer = cx.new(|cx| PromptComposer::new(window, cx));
        ConversationView::new(composer, window, cx)
    });
    let dropped = Arc::new(AtomicBool::new(false));

    window
        .update(cx, |view, window, cx| {
            view.load(
                ConversationThread {
                    conversation: conversation(),
                    messages: Vec::new(),
                },
                cx,
            );
            view.start_generation(
                message(1, MessageRole::User, MessageStatus::Complete),
                message(2, MessageRole::Assistant, MessageStatus::Streaming),
                ProviderId::new("demo"),
                Box::pin(DropAwareStream {
                    dropped: Arc::clone(&dropped),
                }),
                window,
                cx,
            );
            let stale_generation = view.generation;
            view.cancel(cx);
            view.push_stream_chunk(stale_generation, MessageId::new(2), "stale", cx);

            let thread = view
                .snapshot()
                .expect("the conversation should remain loaded");
            let assistant = thread
                .messages
                .last()
                .expect("an assistant response should exist");
            assert_eq!(assistant.status, MessageStatus::Stopped);
            assert_eq!(assistant.content, "");
        })
        .expect("the conversation test window should remain open");

    cx.run_until_parked();
    assert!(dropped.load(Ordering::SeqCst));
}

#[gpui_kit::test]
fn superseding_a_generation_rejects_chunks_from_the_old_stream(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let window = cx.open_window(size(px(900.), px(640.)), |window, cx| {
        let composer = cx.new(|cx| PromptComposer::new(window, cx));
        ConversationView::new(composer, window, cx)
    });

    window
        .update(cx, |view, window, cx| {
            view.load(
                ConversationThread {
                    conversation: conversation(),
                    messages: Vec::new(),
                },
                cx,
            );
            view.start_generation(
                message(1, MessageRole::User, MessageStatus::Complete),
                message(2, MessageRole::Assistant, MessageStatus::Streaming),
                ProviderId::new("demo"),
                Box::pin(stream::pending()),
                window,
                cx,
            );
            let stale_generation = view.generation;
            view.start_generation(
                message(3, MessageRole::User, MessageStatus::Complete),
                message(4, MessageRole::Assistant, MessageStatus::Streaming),
                ProviderId::new("demo"),
                Box::pin(stream::pending()),
                window,
                cx,
            );
            view.push_stream_chunk(stale_generation, MessageId::new(2), "stale", cx);

            let thread = view
                .snapshot()
                .expect("the conversation should remain loaded");
            let old_response = thread
                .messages
                .iter()
                .find(|message| message.id == MessageId::new(2))
                .expect("the superseded response should remain visible");
            assert_eq!(old_response.status, MessageStatus::Stopped);
            assert_eq!(old_response.content, "");
            view.cancel(cx);
        })
        .expect("the conversation test window should remain open");
}
