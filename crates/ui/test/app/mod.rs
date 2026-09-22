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

use gpui_kit::component::Root;
use gpui_kit::{AppContext as _, Entity, TestAppContext, WindowHandle, px, size};

mod behavior;
mod finder;
use magenta_application::{ConversationHistory, ProjectCatalog, RegenerateMessage, SendMessage};
use magenta_core::*;

use super::{
    BubblewrapCapability, MainServices, MainView, OpenConversationFinder, PanelState, StorageState,
    WorkRuntimeReadiness, history::Operation,
};

#[derive(Default)]
struct TestPorts {
    fail_initialize: AtomicBool,
    fail_save: AtomicBool,
    fail_models: AtomicBool,
    fail_settings_save: AtomicBool,
    summaries: Mutex<Vec<ConversationSummary>>,
    search_results: Mutex<Option<Vec<ConversationSearchResult>>>,
    loads: Mutex<VecDeque<StorageFuture<ConversationPage>>>,
    around_loads: Mutex<Vec<(ConversationId, MessageSequence)>>,
    saves: Mutex<Vec<Message>>,
    deleted: Mutex<Vec<ConversationId>>,
    requests: AtomicUsize,
    account: Mutex<Option<ProviderAccount>>,
    models: Mutex<Vec<ModelDescriptor>>,
    settings: Mutex<AppSettings>,
    saved_settings: Mutex<Vec<AppSettings>>,
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
        let cached_results = self.search_results.lock().clone();
        if let Some(results) = cached_results {
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
    fn begin_retry(
        &self,
        _: ConversationId,
        _: MessageId,
        _: GenerationConfig,
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
    fn upsert_assistant_trace(&self, _: MessageId, _: AssistantTrace) -> StorageFuture<()> {
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
        if self.fail_models.load(Ordering::SeqCst) {
            return Box::pin(async {
                Err(ProviderError::with_kind(
                    ProviderId::new("openai"),
                    ProviderErrorKind::ServiceUnavailable,
                    std::io::Error::other("model catalog test failure"),
                ))
            });
        }
        let models = self.models.lock().clone();
        Box::pin(async move { Ok(models) })
    }
}

impl ProviderAuthenticator for TestPorts {
    fn restore(&self) -> AuthenticationFuture<Option<ProviderAccount>> {
        let account = self.account.lock().clone();
        Box::pin(async move { Ok(account) })
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
        let settings = self.settings.lock().clone();
        Box::pin(async move { Ok(settings) })
    }

    fn save(&self, settings: AppSettings) -> SettingsFuture<()> {
        if self.fail_settings_save.load(Ordering::SeqCst) {
            return Box::pin(async {
                Err(SettingsError::new(std::io::Error::other(
                    "settings test failure",
                )))
            });
        }
        *self.settings.lock() = settings.clone();
        self.saved_settings.lock().push(settings);
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

impl RepositoryAccess for TestPorts {
    fn status(&self, _: std::path::PathBuf) -> RepositoryFuture<RepositoryStatus> {
        Box::pin(async {
            Ok(RepositoryStatus {
                branch: Some("main".to_owned()),
                detached: false,
                unborn: false,
                changes: Vec::new(),
            })
        })
    }

    fn diff(
        &self,
        _: std::path::PathBuf,
        path: String,
        area: RepositoryDiffArea,
    ) -> RepositoryFuture<RepositoryDiff> {
        Box::pin(async move {
            Ok(RepositoryDiff {
                path,
                area,
                unified_diff: String::new(),
                binary: false,
            })
        })
    }

    fn stage(&self, _: std::path::PathBuf, _: Vec<String>) -> RepositoryFuture<()> {
        Box::pin(async { Ok(()) })
    }

    fn unstage(&self, _: std::path::PathBuf, _: Vec<String>) -> RepositoryFuture<()> {
        Box::pin(async { Ok(()) })
    }

    fn commit(&self, _: std::path::PathBuf, summary: String) -> RepositoryFuture<RepositoryCommit> {
        Box::pin(async move {
            Ok(RepositoryCommit {
                oid: "0123456".to_owned(),
                summary,
            })
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

fn model(id: &str) -> ModelDescriptor {
    ModelDescriptor {
        provider: ProviderId::new("openai"),
        id: ModelId::new(id),
        display_name: id.to_owned(),
        description: None,
        priority: 1,
        default_effort: EffortLevel::Medium,
        supported_efforts: vec![EffortLevel::Medium, EffortLevel::High],
        limits: GenerationLimits::default(),
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
    window_size: gpui_kit::Size<gpui_kit::Pixels>,
) -> TestWindow {
    cx.update(|cx| {
        gpui_kit::init(cx);
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
                    command_catalog: Arc::new(StaticCommandCatalog::default()),
                    settings_store: ports.clone(),
                    agent: None,
                    projects: Some(project_catalog),
                    repository: Some(ports.clone()),
                    work_runtime: WorkRuntimeReadiness::new(BubblewrapCapability::Unavailable),
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
    window_size: gpui_kit::Size<gpui_kit::Pixels>,
) -> TestWindow {
    cx.update(|cx| {
        gpui_kit::init(cx);
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
                    command_catalog: Arc::new(StaticCommandCatalog::default()),
                    settings_store: ports.clone(),
                    agent: None,
                    projects: None,
                    repository: None,
                    work_runtime: WorkRuntimeReadiness::new(BubblewrapCapability::Unavailable),
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
