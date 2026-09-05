use std::{
    cell::RefCell,
    collections::VecDeque,
    rc::Rc,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

use gpui::{AppContext as _, Entity, TestAppContext, WindowHandle, px, size};
use gpui_component::Root;
use magenta_application::{ConversationHistory, RegenerateMessage, SendMessage};
use magenta_core::*;

use super::{MainView, history::Operation};

#[derive(Default)]
struct TestPorts {
    fail_initialize: AtomicBool,
    fail_save: AtomicBool,
    loads: Mutex<VecDeque<StorageFuture<ConversationPage>>>,
    saves: Mutex<Vec<Message>>,
    requests: AtomicUsize,
}

fn failure<T: Send + 'static>() -> StorageFuture<T> {
    Box::pin(async {
        Err(StorageError::new(
            StorageErrorKind::Unavailable,
            std::io::Error::other("test failure"),
        ))
    })
}

impl ConversationStore for TestPorts {
    fn initialize(&self) -> StorageFuture<()> {
        if self.fail_initialize.load(Ordering::SeqCst) {
            failure()
        } else {
            Box::pin(async { Ok(()) })
        }
    }
    fn summaries(&self) -> StorageFuture<Vec<ConversationSummary>> {
        Box::pin(async { Ok(Vec::new()) })
    }
    fn load(&self, _: ConversationId) -> StorageFuture<ConversationPage> {
        self.loads
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(failure)
    }
    fn earlier(&self, _: ConversationId, _: MessageSequence) -> StorageFuture<MessagePage> {
        failure()
    }
    fn begin_turn(&self, _: BeginTurn) -> StorageFuture<PreparedTurn> {
        failure()
    }
    fn begin_regeneration(&self, _: ConversationId, _: MessageId) -> StorageFuture<PreparedTurn> {
        failure()
    }
    fn finalize(&self, message: Message) -> StorageFuture<()> {
        self.saves.lock().unwrap().push(message);
        if self.fail_save.load(Ordering::SeqCst) {
            failure()
        } else {
            Box::pin(async { Ok(()) })
        }
    }
    fn set_pinned(&self, _: ConversationId, _: bool) -> StorageFuture<()> {
        failure()
    }
}

impl ChatProvider for TestPorts {
    fn stream(&self, _: GenerationRequest) -> GenerationStream {
        self.requests.fetch_add(1, Ordering::SeqCst);
        Box::pin(futures_util::stream::empty())
    }
}

impl ModelCatalog for TestPorts {
    fn models(&self) -> ModelCatalogFuture {
        Box::pin(async { Ok(Vec::new()) })
    }
}

impl ProviderAuthenticator for TestPorts {
    fn restore(&self) -> AuthenticationFuture<Option<ProviderAccount>> {
        Box::pin(async { Ok(None) })
    }
    fn begin_login(&self) -> AuthenticationFuture<AuthorizationSession> {
        Box::pin(std::future::pending())
    }
    fn sign_out(&self) -> AuthenticationFuture<()> {
        Box::pin(async { Ok(()) })
    }
}

fn page(id: u64) -> ConversationPage {
    ConversationPage {
        conversation: Conversation {
            id: ConversationId(id),
            title: format!("Thread {id}"),
            generation: GenerationConfig::new(
                ProviderId::new("test"),
                ModelId::new("model"),
                EffortLevel::Medium,
            ),
        },
        page: MessagePage {
            messages: Vec::new(),
            older_cursor: None,
            has_older: false,
        },
    }
}

fn setup(cx: &mut TestAppContext, ports: Arc<TestPorts>) -> (WindowHandle<Root>, Entity<MainView>) {
    cx.update(gpui_component::init);
    let slot = Rc::new(RefCell::new(None));
    let view_slot = Rc::clone(&slot);
    let window = cx.open_window(size(px(1000.), px(700.)), move |window, cx| {
        let view = cx.new(|cx| {
            MainView::new(
                SendMessage::new(ports.clone(), ports.clone()),
                RegenerateMessage::new(ports.clone(), ports.clone()),
                ConversationHistory::new(ports.clone()),
                ports.clone(),
                ports,
                window,
                cx,
            )
        });
        view_slot.replace(Some(view.clone()));
        Root::new(view, window, cx)
    });
    let view = slot.borrow().clone().unwrap();
    (window, view)
}

#[gpui::test]
fn initialization_failure_can_be_retried_without_demo_history(cx: &mut TestAppContext) {
    let ports = Arc::new(TestPorts::default());
    ports.fail_initialize.store(true, Ordering::SeqCst);
    let (window, view) = setup(cx, ports.clone());
    cx.run_until_parked();
    view.read_with(cx, |main, _| {
        assert!(!main.storage_ready);
        assert!(main.active_conversation.is_none());
    });
    ports.fail_initialize.store(false, Ordering::SeqCst);
    window
        .update(cx, |_, window, cx| {
            view.update(cx, |main, cx| main.load_history(window, cx));
        })
        .unwrap();
    cx.run_until_parked();
    assert!(view.read_with(cx, |main, _| main.storage_ready));
}

#[gpui::test]
fn stale_and_failed_loads_keep_the_correct_selection(cx: &mut TestAppContext) {
    let ports = Arc::new(TestPorts::default());
    let (sender, receiver) = futures_channel::oneshot::channel();
    ports
        .loads
        .lock()
        .unwrap()
        .push_back(Box::pin(async move { receiver.await.unwrap() }));
    ports
        .loads
        .lock()
        .unwrap()
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

#[gpui::test]
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
    assert_eq!(
        ports.saves.lock().unwrap().as_slice(),
        &[response.clone(), response]
    );
}
