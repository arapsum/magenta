use parking_lot::Mutex;
use std::{
    cell::RefCell,
    collections::VecDeque,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

use gpui::{AppContext as _, Entity, TestAppContext, WindowHandle, px, size};
use gpui_component::Root;

mod finder;
use magenta_application::{ConversationHistory, ProjectCatalog, RegenerateMessage, SendMessage};
use magenta_core::*;

use super::{
    MainServices, MainView, OpenConversationFinder, PanelState, StorageState, history::Operation,
};

#[derive(Default)]
struct TestPorts {
    fail_initialize: AtomicBool,
    fail_save: AtomicBool,
    summaries: Mutex<Vec<ConversationSummary>>,
    search_results: Mutex<Option<Vec<ConversationSearchResult>>>,
    loads: Mutex<VecDeque<StorageFuture<ConversationPage>>>,
    around_loads: Mutex<Vec<(ConversationId, MessageSequence)>>,
    saves: Mutex<Vec<Message>>,
    deleted: Mutex<Vec<ConversationId>>,
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
        let summaries = self.summaries.lock().clone();
        Box::pin(async move { Ok(summaries) })
    }
    fn search(&self, query: String, limit: usize) -> StorageFuture<Vec<ConversationSearchResult>> {
        if let Some(results) = self.search_results.lock().clone() {
            return Box::pin(async move { Ok(results) });
        }
        let query = query.to_lowercase();
        let results = self
            .summaries
            .lock()
            .iter()
            .filter(|summary| {
                summary.title.to_lowercase().contains(&query)
                    || summary.preview.to_lowercase().contains(&query)
            })
            .take(limit)
            .map(|summary| ConversationSearchResult {
                conversation_id: summary.id,
                message_id: None,
                message_sequence: None,
                title: summary.title.clone(),
                title_highlights: Vec::new(),
                snippet: summary.preview.clone(),
                snippet_highlights: Vec::new(),
                updated_at: summary.updated_at,
            })
            .collect();
        Box::pin(async move { Ok(results) })
    }
    fn load(&self, _: ConversationId) -> StorageFuture<ConversationPage> {
        self.loads.lock().pop_front().unwrap_or_else(failure)
    }
    fn load_around(
        &self,
        id: ConversationId,
        sequence: MessageSequence,
    ) -> StorageFuture<ConversationPage> {
        self.around_loads.lock().push((id, sequence));
        self.loads.lock().pop_front().unwrap_or_else(failure)
    }
    fn earlier(&self, _: ConversationId, _: MessageSequence) -> StorageFuture<MessagePage> {
        failure()
    }
    fn later(&self, _: ConversationId, _: MessageSequence) -> StorageFuture<MessagePage> {
        failure()
    }
    fn begin_turn(&self, _: BeginTurn) -> StorageFuture<PreparedTurn> {
        failure()
    }
    fn begin_regeneration(
        &self,
        _: ConversationId,
        _: MessageId,
        _: u64,
    ) -> StorageFuture<PreparedTurn> {
        failure()
    }
    fn finalize(&self, message: Message) -> StorageFuture<()> {
        self.saves.lock().push(message);
        if self.fail_save.load(Ordering::SeqCst) {
            failure()
        } else {
            Box::pin(async { Ok(()) })
        }
    }
    fn delete(&self, id: ConversationId) -> StorageFuture<()> {
        self.deleted.lock().push(id);
        self.summaries.lock().retain(|summary| summary.id != id);
        Box::pin(async { Ok(()) })
    }
    fn rename(&self, _: ConversationId, _: String) -> StorageFuture<()> {
        failure()
    }
    fn rename_if_current(&self, _: ConversationId, _: String, _: String) -> StorageFuture<bool> {
        Box::pin(async { Ok(true) })
    }
    fn set_pinned(&self, _: ConversationId, _: bool) -> StorageFuture<()> {
        failure()
    }
    fn append_agent_activity(&self, _: AgentActivityRecord) -> StorageFuture<()> {
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

impl SettingsStore for TestPorts {
    fn load(&self) -> SettingsFuture<AppSettings> {
        Box::pin(async { Ok(AppSettings::default()) })
    }

    fn save(&self, _: AppSettings) -> SettingsFuture<()> {
        Box::pin(async { Ok(()) })
    }

    fn reset(&self) -> SettingsFuture<AppSettings> {
        Box::pin(async { Ok(AppSettings::default()) })
    }

    fn path(&self) -> std::path::PathBuf {
        std::path::PathBuf::from("settings.toml")
    }
}

impl ProjectStore for TestPorts {
    fn projects(&self) -> StorageFuture<Vec<Project>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    fn upsert_project(&self, _: Project) -> StorageFuture<()> {
        Box::pin(async { Ok(()) })
    }

    fn remove_project(&self, _: std::path::PathBuf) -> StorageFuture<()> {
        Box::pin(async { Ok(()) })
    }
}

impl WorkspaceBrowser for TestPorts {
    fn canonicalize_root(&self, root: std::path::PathBuf) -> WorkspaceFuture<std::path::PathBuf> {
        Box::pin(async move { Ok(root) })
    }

    fn list_directory(
        &self,
        _: std::path::PathBuf,
        _: String,
    ) -> WorkspaceFuture<Vec<WorkspaceEntry>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    fn read_document(
        &self,
        _: std::path::PathBuf,
        _: String,
    ) -> WorkspaceFuture<WorkspaceDocument> {
        Box::pin(async {
            Err(WorkspaceError::new(std::io::Error::other(
                "test document unavailable",
            )))
        })
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
            mode: ConversationMode::Chat,
            workspace_root: None,
        },
        page: MessagePage {
            messages: Vec::new(),
            older_cursor: None,
            has_older: false,
            newer_cursor: None,
            has_newer: false,
        },
    }
}

fn summary(id: u64, title: &str) -> ConversationSummary {
    ConversationSummary {
        id: ConversationId(id),
        title: title.to_owned(),
        preview: format!("Preview for {title}"),
        pinned: false,
        mode: ConversationMode::Chat,
        workspace_root: None,
        provider: ProviderId::new("openai"),
        created_at: Timestamp(0),
        updated_at: Timestamp(id.cast_signed()),
    }
}

type TestWindow = (WindowHandle<Root>, Entity<MainView>);

fn setup(cx: &mut TestAppContext, ports: Arc<TestPorts>) -> TestWindow {
    setup_at(cx, ports, size(px(1000.), px(700.)))
}

fn setup_with_projects(cx: &mut TestAppContext, ports: Arc<TestPorts>) -> TestWindow {
    setup_with_projects_at(cx, ports, size(px(1000.), px(700.)))
}

fn setup_with_projects_at(
    cx: &mut TestAppContext,
    ports: Arc<TestPorts>,
    window_size: gpui::Size<gpui::Pixels>,
) -> TestWindow {
    cx.update(|cx| {
        gpui_component::init(cx);
        crate::settings::init(cx);
    });
    let slot = Rc::new(RefCell::new(None));
    let view_slot = Rc::clone(&slot);
    let window = cx.open_window(window_size, move |window, cx| {
        let project_catalog = ProjectCatalog::new(ports.clone(), ports.clone());
        let view = cx.new(|cx| {
            MainView::new(
                SendMessage::new(ports.clone(), ports.clone()),
                RegenerateMessage::new(ports.clone(), ports.clone()),
                ConversationHistory::new(ports.clone()),
                MainServices {
                    authenticator: ports.clone(),
                    model_catalog: ports.clone(),
                    settings_store: ports.clone(),
                    agent: None,
                    projects: Some(project_catalog),
                },
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

fn setup_at(
    cx: &mut TestAppContext,
    ports: Arc<TestPorts>,
    window_size: gpui::Size<gpui::Pixels>,
) -> TestWindow {
    cx.update(|cx| {
        gpui_component::init(cx);
        crate::settings::init(cx);
    });
    let slot = Rc::new(RefCell::new(None));
    let view_slot = Rc::clone(&slot);
    let window = cx.open_window(window_size, move |window, cx| {
        let view = cx.new(|cx| {
            MainView::new(
                SendMessage::new(ports.clone(), ports.clone()),
                RegenerateMessage::new(ports.clone(), ports.clone()),
                ConversationHistory::new(ports.clone()),
                MainServices {
                    authenticator: ports.clone(),
                    model_catalog: ports.clone(),
                    settings_store: ports.clone(),
                    agent: None,
                    projects: None,
                },
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
fn new_chat_layout_keeps_primary_content_visible(cx: &mut TestAppContext) {
    let ports = Arc::new(TestPorts::default());
    ports
        .summaries
        .lock()
        .extend([summary(1, "A recent thread"), summary(2, "Another thread")]);
    let (window, _) = setup(cx, ports);
    cx.run_until_parked();

    let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
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

#[gpui::test]
fn narrow_new_chat_layout_keeps_the_composer_in_view(cx: &mut TestAppContext) {
    let (window, _) = setup_at(cx, Arc::new(TestPorts::default()), size(px(680.), px(640.)));
    cx.run_until_parked();

    let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
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

#[gpui::test]
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

#[gpui::test]
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

#[gpui::test]
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
        agent_activities: Vec::new(),
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

#[gpui::test]
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
                    sidebar.set_active(Some(ConversationId(1)), cx)
                });
                main.confirm_delete_conversation(ConversationId(1), window, cx);
            });
        })
        .unwrap();

    let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
    visual.run_until_parked();
    assert!(view.read_with(cx, |main, _| main.pending_deletion.is_some()));

    let delete_bounds = visual
        .debug_bounds("confirm-delete-conversation")
        .expect("the delete confirmation button should be rendered");
    visual.simulate_click(delete_bounds.center(), gpui::Modifiers::default());
    visual.run_until_parked();

    assert_eq!(ports.deleted.lock().as_slice(), &[ConversationId(1)]);
    view.read_with(cx, |main, _| {
        assert!(main.active_conversation.is_none());
        assert!(main.pending_deletion.is_none());
        assert_eq!(main.operation, Operation::Idle);
    });
}
