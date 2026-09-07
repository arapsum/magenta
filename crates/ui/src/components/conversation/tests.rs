use std::{
    cell::RefCell,
    path::PathBuf,
    pin::Pin,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context as TaskContext, Poll},
};

use futures_util::{Stream, stream};
use gpui::{TestAppContext, size};
use gpui_component::Root;
use magenta_core::{
    ConversationId, EffortLevel, FinishReason, GenerationConfig, ModelId, TokenUsage,
};

use super::*;

fn conversation() -> Conversation {
    Conversation {
        id: ConversationId::new(77),
        title: "Provider-ready stream".to_owned(),
        generation: GenerationConfig::new(
            ProviderId::new("demo"),
            ModelId::new("magenta-demo"),
            EffortLevel::Medium,
        ),
    }
}

fn stored_page(range: std::ops::Range<u64>) -> magenta_core::MessagePage {
    let has_older = range.start > 0;
    let messages = range
        .map(|id| magenta_core::StoredMessage {
            message: message(id, MessageRole::Assistant, MessageStatus::Complete),
            sequence: magenta_core::MessageSequence(i64::try_from(id).unwrap()),
            created_at: magenta_core::Timestamp(0),
            generation: conversation().generation,
        })
        .collect::<Vec<_>>();
    magenta_core::MessagePage {
        older_cursor: messages.first().map(|message| message.sequence),
        messages,
        has_older,
    }
}

#[test]
fn elapsed_time_uses_compact_stable_units() {
    assert_eq!(format_elapsed(Duration::ZERO), "0s");
    assert_eq!(format_elapsed(Duration::from_secs(59)), "59s");
    assert_eq!(format_elapsed(Duration::from_secs(60)), "1m 00s");
    assert_eq!(format_elapsed(Duration::from_secs(3_725)), "1h 02m");
}

#[gpui::test]
fn attachment_preview_can_be_opened_and_dismissed(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let window = cx.open_window(size(px(900.), px(640.)), |window, cx| {
        let composer = cx.new(|cx| PromptComposer::new(window, cx));
        ConversationView::new(composer, window, cx)
    });

    window
        .update(cx, |view, _, cx| {
            view.attachment_preview = Some(AttachmentPreview {
                path: PathBuf::from("reference.png"),
                name: "Reference image".to_owned(),
            });
            view.close_attachment_preview(cx);
            assert!(view.attachment_preview.is_none());
        })
        .expect("the conversation test window should remain open");
}

#[gpui::test]
fn generation_progress_tracks_stream_phases_and_clears_on_stop(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
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

            let generation = view.generation;
            let progress = view
                .generation_progress
                .as_ref()
                .expect("generation progress should start immediately");
            assert_eq!(progress.phase, GenerationPhase::Connecting);
            assert!(progress.provider_started_at.is_none());
            assert!(progress.first_text_at.is_none());

            view.mark_provider_started(generation, MessageId::new(2), cx);
            let progress = view.generation_progress.as_ref().unwrap();
            assert_eq!(progress.phase, GenerationPhase::Thinking);
            assert!(progress.provider_started_at.is_some());

            view.push_stream_chunk(generation, MessageId::new(2), "first", cx);
            let first_text_at = view
                .generation_progress
                .as_ref()
                .and_then(|progress| progress.first_text_at)
                .expect("the first text timestamp should be recorded");
            assert_eq!(
                view.generation_progress.as_ref().unwrap().phase,
                GenerationPhase::Responding
            );

            view.push_stream_chunk(generation, MessageId::new(2), " second", cx);
            assert_eq!(
                view.generation_progress
                    .as_ref()
                    .and_then(|progress| progress.first_text_at),
                Some(first_text_at)
            );

            view.cancel(cx);
            assert!(view.generation_progress.is_none());
            assert!(view.generation_clock_task.is_none());
        })
        .expect("the conversation test window should remain open");
}

#[gpui::test]
fn prepending_history_preserves_visible_message_and_offset(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let window = cx.open_window(size(px(900.), px(640.)), |window, cx| {
        let composer = cx.new(|cx| PromptComposer::new(window, cx));
        ConversationView::new(composer, window, cx)
    });
    window
        .update(cx, |view, _, cx| {
            view.load_page(
                magenta_core::ConversationPage {
                    conversation: conversation(),
                    page: stored_page(50..100),
                },
                cx,
            );
            view.list_state.scroll_to(gpui::ListOffset {
                item_ix: 10,
                offset_in_item: px(7.),
            });
            view.prepend_page(stored_page(0..50), cx);
            let anchor = view.list_state.logical_scroll_top();
            assert_eq!(anchor.item_ix, 60);
            assert_eq!(anchor.offset_in_item, px(7.));
            assert_eq!(view.messages[anchor.item_ix].message.id, MessageId(60));
            assert!(!view.has_older);
            view.prepend_page(stored_page(0..50), cx);
            assert_eq!(view.messages.len(), 100);
            assert_eq!(view.list_state.logical_scroll_top().item_ix, 60);
        })
        .unwrap();
}

#[gpui::test]
fn stream_deltas_do_not_emit_persistence_events(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let saved = Rc::new(RefCell::new(Vec::new()));
    let events = Rc::clone(&saved);
    let window = cx.open_window(size(px(900.), px(640.)), |window, cx| {
        let composer = cx.new(|cx| PromptComposer::new(window, cx));
        ConversationView::new(composer, window, cx)
    });
    let subscription = window
        .update(cx, |view, window, cx| {
            let subscription = cx.subscribe(&cx.entity(), move |_, _, event, _| {
                if let ConversationViewEvent::GenerationFinished(message) = event {
                    events.borrow_mut().push(message.clone());
                }
            });
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
                ProviderId::new("test"),
                Box::pin(stream::pending()),
                window,
                cx,
            );
            for _ in 0..100 {
                view.push_stream_chunk(view.generation, MessageId(2), "x", cx);
            }
            subscription
        })
        .unwrap();
    cx.run_until_parked();
    assert!(saved.borrow().is_empty());
    window.update(cx, |view, _, cx| view.cancel(cx)).unwrap();
    cx.run_until_parked();
    assert_eq!(saved.borrow().len(), 1);
    assert_eq!(saved.borrow()[0].content.len(), 100);
    assert_eq!(saved.borrow()[0].status, MessageStatus::Stopped);
    drop(subscription);
}

fn message(id: u64, role: MessageRole, status: MessageStatus) -> Message {
    Message {
        id: MessageId::new(id),
        conversation_id: ConversationId::new(77),
        role,
        content: String::new(),
        status,
        attachments: Vec::new(),
        generation_outcome: None,
    }
}

struct DropAwareStream {
    dropped: Arc<AtomicBool>,
}

impl Stream for DropAwareStream {
    type Item = Result<GenerationEvent, ProviderError>;

    fn poll_next(self: Pin<&mut Self>, _cx: &mut TaskContext<'_>) -> Poll<Option<Self::Item>> {
        Poll::Pending
    }
}

impl Drop for DropAwareStream {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::SeqCst);
    }
}

#[gpui::test]
fn loading_a_fixture_keeps_the_conversation_in_the_view(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let window = cx.open_window(size(px(900.), px(640.)), |window, cx| {
        let composer = cx.new(|cx| PromptComposer::new(window, cx));
        ConversationView::new(composer, window, cx)
    });

    window
        .update(cx, |view, _, cx| {
            let thread = ConversationThread {
                conversation: conversation(),
                messages: vec![message(1, MessageRole::User, MessageStatus::Complete)],
            };
            view.load(thread, cx);
            assert!(view.snapshot().is_some_and(|thread| {
                thread.conversation.id == magenta_core::ConversationId::new(77)
                    && !thread.messages.is_empty()
            }));
        })
        .expect("the conversation test window should remain open");
}

#[gpui::test]
fn user_messages_keep_prose_literal_and_isolate_fenced_code(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let window = cx.open_window(size(px(900.), px(640.)), |window, cx| {
        let composer = cx.new(|cx| PromptComposer::new(window, cx));
        ConversationView::new(composer, window, cx)
    });

    window
        .update(cx, |_, _, cx| {
            let message = Message {
                id: MessageId::new(7),
                conversation_id: magenta_core::ConversationId::new(3),
                role: MessageRole::User,
                content: "Before\n```rust\nlet answer = 42;\n```\nAfter".to_owned(),
                status: MessageStatus::Complete,
                attachments: Vec::new(),
                generation_outcome: None,
            };
            let rendered = ConversationView::rendered_message(message, cx);

            assert!(matches!(
                &rendered.user_segments[0],
                RenderedUserSegment::Text(text) if text == "Before\n"
            ));
            assert!(matches!(
                &rendered.user_segments[1],
                RenderedUserSegment::Code {
                    source_start: 7,
                    ..
                }
            ));
            assert!(matches!(
                &rendered.user_segments[2],
                RenderedUserSegment::Text(text) if text == "After"
            ));
        })
        .expect("the conversation test window should remain open");
}

#[gpui::test]
fn completed_stream_stores_its_outcome(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
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

#[gpui::test]
fn stream_ending_without_completion_marks_the_response_failed(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
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

#[gpui::test]
fn cancellation_drops_the_stream_and_rejects_stale_chunks(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
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
            assert!(assistant.content.is_empty());
        })
        .expect("the conversation test window should remain open");

    cx.run_until_parked();
    assert!(dropped.load(Ordering::SeqCst));
}

#[gpui::test]
fn superseding_a_generation_rejects_chunks_from_the_old_stream(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
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
            assert!(old_response.content.is_empty());
            view.cancel(cx);
        })
        .expect("the conversation test window should remain open");
}
