mod diagnostics;

use std::{borrow::Cow, cell::Cell, process::ExitCode, rc::Rc, sync::Arc};

use gpui::{
    px, size, App, AppContext, Application, AssetSource, Bounds, Result as GpuiResult,
    SharedString, WindowBounds, WindowHandle, WindowOptions,
};
#[cfg(target_os = "linux")]
use gpui::{WindowBackgroundAppearance, WindowDecorations};
use gpui_component::Root;
use magenta_application::{ConversationHistory, RegenerateMessage, SendMessage};
use magenta_core::{
    ChatProvider, ConversationStore, ModelCatalog, ProviderAuthenticator, SettingsStore,
};
use magenta_providers::OpenAiProvider;

use magenta_ui::{notification_for_error, MagentaError, MainServices, MainView, Result};

struct MagentaAssets;

impl AssetSource for MagentaAssets {
    fn load(&self, path: &str) -> GpuiResult<Option<Cow<'static, [u8]>>> {
        let asset = match path {
            "icons/conversation-pin.svg" => {
                Some(include_bytes!("../assets/icons/conversation-pin.svg").as_slice())
            }
            "icons/conversation-rename.svg" => {
                Some(include_bytes!("../assets/icons/conversation-rename.svg").as_slice())
            }
            "icons/conversation-delete.svg" => {
                Some(include_bytes!("../assets/icons/conversation-delete.svg").as_slice())
            }
            "icons/generation-stop.svg" => {
                Some(include_bytes!("../assets/icons/generation-stop.svg").as_slice())
            }
            _ => None,
        };

        asset.map_or_else(
            || gpui_component_assets::Assets.load(path),
            |asset| Ok(Some(Cow::Borrowed(asset))),
        )
    }

    fn list(&self, path: &str) -> GpuiResult<Vec<SharedString>> {
        let mut assets = gpui_component_assets::Assets.list(path)?;
        if path.is_empty() || path == "icons" {
            assets.extend([
                "icons/conversation-pin.svg".into(),
                "icons/conversation-rename.svg".into(),
                "icons/conversation-delete.svg".into(),
                "icons/generation-stop.svg".into(),
            ]);
        }
        Ok(assets)
    }
}

fn main() -> ExitCode {
    let (diagnostics_guard, diagnostics_error) = initialize_diagnostics();
    diagnostics::install_panic_hook();

    let launch_failed = Rc::new(Cell::new(false));
    run_application(diagnostics_error, Rc::clone(&launch_failed));

    drop(diagnostics_guard);
    exit_code(launch_failed.get())
}

fn initialize_diagnostics() -> (Option<diagnostics::DiagnosticsGuard>, Option<MagentaError>) {
    match diagnostics::init() {
        Ok(guard) => (Some(guard), None),
        Err(error) => {
            eprintln!("Magenta could not create its local diagnostics file; using stderr.");
            diagnostics::init_stderr();
            tracing::warn!(
                error = ?error,
                code = error.presentation().code,
                operation = "diagnostics.initialize",
                "local diagnostics unavailable"
            );
            (None, Some(error))
        }
    }
}

fn run_application(diagnostics_error: Option<MagentaError>, launch_failed: Rc<Cell<bool>>) {
    let app: Application = gpui_platform::application().with_assets(MagentaAssets);

    app.run(move |cx: &mut App| {
        initialize_application(cx);
        let startup_warnings = collect_startup_warnings(diagnostics_error, cx);
        launch_main_window(startup_warnings, &launch_failed, cx);
    });
}

fn initialize_application(cx: &mut App) {
    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        os = std::env::consts::OS,
        architecture = std::env::consts::ARCH,
        operation = "application.start",
        "starting Magenta"
    );
    cx.set_app_identity("magenta-1", "Magenta");
    gpui_component::init(cx);
    magenta_ui::init_settings(cx);
}

fn collect_startup_warnings(
    diagnostics_error: Option<MagentaError>,
    cx: &mut App,
) -> Vec<MagentaError> {
    diagnostics_error
        .into_iter()
        .chain(theme_startup_warning(cx))
        .collect()
}

fn theme_startup_warning(cx: &mut App) -> Option<MagentaError> {
    let error = magenta_ui::theme::init(cx).into_error()?;
    tracing::warn!(
        error = ?error,
        code = error.presentation().code,
        operation = "theme.initialize",
        "using the default dark theme"
    );
    Some(error)
}

fn launch_main_window(
    startup_warnings: Vec<MagentaError>,
    launch_failed: &Cell<bool>,
    cx: &mut App,
) {
    match open_main_window(cx) {
        Ok(window_handle) => {
            present_startup_warnings(window_handle, startup_warnings, cx);
            tracing::info!(operation = "window.open", "main window opened");
        }
        Err(error) => {
            tracing::error!(
                error = ?error,
                code = error.presentation().code,
                operation = "window.open",
                "main window creation failed"
            );
            launch_failed.set(true);
            cx.quit();
        }
    }
}

const fn exit_code(launch_failed: bool) -> ExitCode {
    if launch_failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

fn open_main_window(cx: &mut App) -> Result<WindowHandle<Root>> {
    let window_options = main_window_options(cx);
    let provider = Arc::new(OpenAiProvider::new());
    let chat_provider: Arc<dyn ChatProvider> = provider.clone();
    let authenticator: Arc<dyn ProviderAuthenticator> = provider.clone();
    let model_catalog: Arc<dyn ModelCatalog> = provider;
    let data_dir = dirs::data_local_dir().ok_or_else(|| MagentaError::StorageInitialize {
        source: magenta_core::StorageError::new(
            magenta_core::StorageErrorKind::Unavailable,
            std::io::Error::other("local data directory unavailable"),
        ),
    })?;
    let store: Arc<dyn ConversationStore> =
        Arc::new(magenta_storage::SqliteConversationStore::new(
            data_dir.join("magenta/conversations.sqlite3"),
        ));
    let config_dir = dirs::config_dir().ok_or_else(|| MagentaError::StorageInitialize {
        source: magenta_core::StorageError::new(
            magenta_core::StorageErrorKind::Unavailable,
            std::io::Error::other("configuration directory unavailable"),
        ),
    })?;
    let settings_store: Arc<dyn SettingsStore> = Arc::new(magenta_storage::TomlSettingsStore::new(
        config_dir.join("magenta/settings.toml"),
    ));
    let send_message = SendMessage::new(Arc::clone(&chat_provider), Arc::clone(&store));
    let regenerate_provider = Arc::clone(&chat_provider);
    let regenerate_store = Arc::clone(&store);
    let regenerate_message = RegenerateMessage::new(regenerate_provider, regenerate_store);
    let history = ConversationHistory::new(store);
    cx.open_window(window_options, move |window, cx| {
        let main_view = cx.new(|cx| {
            MainView::new(
                send_message,
                regenerate_message,
                history,
                MainServices {
                    authenticator: Arc::clone(&authenticator),
                    model_catalog: Arc::clone(&model_catalog),
                    settings_store: Arc::clone(&settings_store),
                },
                window,
                cx,
            )
        });
        cx.new(|cx| Root::new(main_view, window, cx))
    })
    .map_err(|source| MagentaError::WindowOpen { source })
}

fn main_window_options(cx: &App) -> WindowOptions {
    let bounds = Bounds::centered(None, size(px(1180.), px(760.)), cx);
    let mut window_options = gpui_component::TitleBar::window_options();
    window_options.window_bounds = Some(WindowBounds::Windowed(bounds));
    window_options.app_id = Some("magenta-1".into());
    configure_titlebar(&mut window_options);
    #[cfg(target_os = "linux")]
    configure_linux_window(&mut window_options);
    window_options
}

fn configure_titlebar(window_options: &mut WindowOptions) {
    if let Some(titlebar) = window_options.titlebar.as_mut() {
        titlebar.title = Some("Magenta".into());
    } else {
        tracing::warn!(
            operation = "window.configure",
            "GPUI did not provide titlebar options; using platform defaults"
        );
    }
}

#[cfg(target_os = "linux")]
const fn configure_linux_window(window_options: &mut WindowOptions) {
    window_options.window_decorations = Some(WindowDecorations::Client);
    window_options.window_background = WindowBackgroundAppearance::Transparent;
}

fn present_startup_warnings(
    window_handle: WindowHandle<Root>,
    warnings: Vec<MagentaError>,
    cx: &mut App,
) {
    if warnings.is_empty() {
        return;
    }

    if let Err(error) = window_handle.update(cx, move |root, window, cx| {
        for warning in &warnings {
            root.push_notification(notification_for_error(warning), window, cx);
        }
    }) {
        tracing::error!(
            error = ?error,
            operation = "startup-warning.present",
            "could not present startup warnings"
        );
    }
}
