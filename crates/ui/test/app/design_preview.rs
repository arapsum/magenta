//! Opt-in renders of the real application with isolated, in-memory fixtures.
//! Run with `cargo test -p magenta-ui export_design_previews -- --ignored`.
use super::*;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AssetSource, HeadlessAppContext, SharedString};
use std::{borrow::Cow, path::PathBuf};

struct PreviewAssets;

impl AssetSource for PreviewAssets {
    fn load(&self, path: &str) -> gpui_kit::Result<Option<Cow<'static, [u8]>>> {
        let local = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../desktop/assets")
            .join(path);
        if let Ok(bytes) = std::fs::read(local) {
            return Ok(Some(Cow::Owned(bytes)));
        }
        crate::UiIcons.load(path)?.map_or_else(
            || gpui_kit::assets::Assets.load(path),
            |icon| Ok(Some(icon)),
        )
    }

    fn list(&self, path: &str) -> gpui_kit::Result<Vec<SharedString>> {
        let mut assets = gpui_kit::assets::Assets.list(path)?;
        assets.extend(crate::UiIcons.list(path)?);
        Ok(assets)
    }
}

fn preview(
    appearance: AppearanceMode,
    viewport: (f32, f32),
    setup: bool,
) -> (HeadlessAppContext, WindowHandle<Root>, Entity<MainView>) {
    let mut cx = HeadlessAppContext::with_platform(
        gpui_kit::platform::current_platform(true).text_system(),
        Arc::new(PreviewAssets),
        gpui_kit::platform::current_headless_renderer,
    );
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::init(cx);
        crate::settings::init(cx);
    });
    let ports = Arc::new(TestPorts::default());
    {
        let mut settings = ports.settings.lock();
        settings.appearance = appearance;
        settings.onboarding.set_setup_acknowledged(!setup);
    }
    *ports.account.lock() = Some(ProviderAccount {
        provider: ProviderId::new("openai"),
        name: Some("Alex Morgan".to_owned()),
        email: Some("alex@example.com".to_owned()),
        plan: Some("plus".to_owned()),
    });
    ports.models.lock().push(model("GPT-5.6"));
    let now = chrono::Utc::now().timestamp_millis();
    for (index, title) in [
        "A calmer way to work",
        "Notes on a new direction",
        "The shape of a good idea",
        "Understanding async Rust",
        "A plan for the week",
    ]
    .into_iter()
    .enumerate()
    {
        let mut item = summary(index as u64 + 1, title);
        item.updated_at = Timestamp(now - i64::try_from(index).unwrap() * 3_600_000);
        item.created_at = item.updated_at;
        item.pinned = index < 2;
        ports.summaries.lock().push(item);
    }
    let slot = Rc::new(RefCell::new(None));
    let view_slot = slot.clone();
    let handle = cx
        .open_window(size(px(viewport.0), px(viewport.1)), move |window, cx| {
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
                        repository: Some(ports),
                        work_runtime: WorkRuntimeReadiness::new(BubblewrapCapability::Unavailable),
                    },
                    window,
                    cx,
                )
            });
            view_slot.replace(Some(view.clone()));
            cx.new(|cx| Root::new(view, window, cx))
        })
        .expect("open the preview window");
    cx.run_until_parked();
    let view = slot.borrow().clone().unwrap();
    (cx, handle, view)
}

fn save(cx: &mut HeadlessAppContext, handle: WindowHandle<Root>, name: &str) {
    let destination = std::env::var_os("MAGENTA_PREVIEW_DIR").map_or_else(
        || PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/design-preview"),
        PathBuf::from,
    );
    std::fs::create_dir_all(&destination).unwrap();
    // Flush layout and asynchronous SVG rasterization before reading GPU pixels.
    for _ in 0..4 {
        cx.run_until_parked();
        cx.update_window(handle.into(), |_, window, cx| window.render_frame(cx))
            .unwrap();
    }
    let path = destination.join(format!("{name}.png"));
    cx.capture_screenshot(handle.into())
        .unwrap()
        .save(&path)
        .unwrap();
    eprintln!("Rendered {}", path.display());
}

fn populate_conversation(cx: &mut HeadlessAppContext, view: &Entity<MainView>) {
    use crate::components::conversation::ConversationThread;
    let generation = GenerationConfig::new(
        ProviderId::new("openai"),
        ModelId::new("GPT-5.6"),
        EffortLevel::Medium,
    );
    let messages = [
        (MessageRole::User, "How can we make this workspace feel more focused?"),
        (MessageRole::Assistant, "Start with a clear reading column, then let the controls recede until they’re needed.\n\n### A calmer starting point\n\n1. **Give the content room.** Keep a consistent measure and a little more space between ideas.\n2. **Create a quiet hierarchy.** Use surface depth for structure and reserve magenta for the moments that matter.\n3. **Keep the next step close.** A simple composer makes it easy to keep thinking.\n\n```rust\nfn next_step(idea: &str) -> String {\n    format!(\"Let’s explore {idea}\")\n}\n```\n\nThe details should help you stay with the thought."),
    ].into_iter().enumerate().map(|(index, (role, content))| Message {
        id: MessageId(index as u64 + 1),
        conversation_id: ConversationId(1),
        role,
        command_id: None,
        content: content.to_owned(),
        status: MessageStatus::Complete,
        attachments: Vec::new(),
        generation_outcome: None,
        failure: None,
        assistant_trace: AssistantTrace::default(),
    }).collect();
    view.update(cx, |main, cx| {
        main.active_conversation = Some(ConversationId(1));
        main.sidebar.update(cx, |sidebar, cx| {
            sidebar.set_active(main.active_conversation, cx);
        });
        main.conversation.update(cx, |conversation, cx| {
            conversation.load(
                ConversationThread {
                    conversation: Conversation {
                        id: ConversationId(1),
                        title: "A calmer way to work".to_owned(),
                        generation,
                        mode: ConversationMode::Chat,
                        workspace_root: None,
                    },
                    messages,
                },
                cx,
            );
        });
        cx.notify();
    });
}

#[test]
#[ignore = "Exports PNG previews using the native text system and a GPU renderer"]
fn export_design_previews() {
    for (appearance, name) in [
        (AppearanceMode::Dark, "dark"),
        (AppearanceMode::Light, "light"),
    ] {
        let (mut cx, handle, view) = preview(appearance, (1180., 800.), false);
        save(&mut cx, handle, &format!("{name}-new-chat"));
        cx.update_window(handle.into(), |_, window, cx| {
            view.update(cx, |main, cx| {
                main.composer.update(cx, |composer, cx| {
                    composer.set_agent_capability(
                        crate::components::prompt_input::AgentCapability::Files,
                        cx,
                    );
                    composer.select_mode(ConversationMode::Agent, window, cx);
                });
            });
        })
        .unwrap();
        save(&mut cx, handle, &format!("{name}-new-work"));
        cx.update_window(handle.into(), |_, window, cx| {
            view.update(cx, |main, cx| {
                main.composer.update(cx, |composer, cx| {
                    composer.select_mode(ConversationMode::Chat, window, cx);
                });
            });
        })
        .unwrap();
        populate_conversation(&mut cx, &view);
        save(&mut cx, handle, &format!("{name}-conversation"));
    }
    let (mut cx, handle, _) = preview(AppearanceMode::Dark, (680., 640.), true);
    save(&mut cx, handle, "dark-compact-setup");

    let (mut cx, handle, view) = preview(AppearanceMode::Dark, (1440., 900.), false);
    populate_conversation(&mut cx, &view);
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |main, cx| {
            main.workbench_open = true;
            let project = Project {
                name: "Magenta".to_owned(),
                root: PathBuf::from("/workspace/magenta"),
                added_at: Timestamp(0),
                last_opened_at: Timestamp(0),
            };
            main.sidebar.update(cx, |sidebar, cx| sidebar.set_projects(vec![project.clone()], cx));
            main.workbench.as_ref().unwrap().update(cx, |workbench, cx| {
                workbench.set_project(Some(project), window, cx);
                workbench.show_change(AgentWorkspaceChange {
                    call_id: "preview".to_owned(),
                    path: "src/workspace.rs".to_owned(),
                    kind: WorkspaceChangeKind::Modify,
                    content: "use std::path::PathBuf;\n\n/// A place to turn ideas into something useful.\npub struct Workspace {\n    name: String,\n    root: PathBuf,\n}\n\nimpl Workspace {\n    pub fn new(name: String, root: PathBuf) -> Self {\n        Self { name, root }\n    }\n\n    pub fn display_name(&self) -> &str {\n        &self.name\n    }\n}\n".to_owned(),
                    diff: String::new(),
                    state: WorkspaceChangeState::Committed,
                    error: None,
                }, window, cx);
            });
            cx.notify();
        });
    }).unwrap();
    save(&mut cx, handle, "dark-workbench");
}
