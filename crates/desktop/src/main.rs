mod diagnostics;

use std::{cell::Cell, process::ExitCode, rc::Rc};

use gpui::{px, size, App, AppContext, Application, Bounds, WindowBounds, WindowHandle};
#[cfg(target_os = "linux")]
use gpui::{WindowBackgroundAppearance, WindowDecorations};
use gpui_component::Root;

use magenta_ui::{notification_for_error, MagentaError, MainView, Result};

fn main() -> ExitCode {
    let (diagnostics_guard, diagnostics_error) = match diagnostics::init() {
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
    };
    diagnostics::install_panic_hook();

    let launch_failed = Rc::new(Cell::new(false));
    let launch_failed_in_app = Rc::clone(&launch_failed);
    let app: Application = gpui_platform::application().with_assets(gpui_component_assets::Assets);

    app.run(move |cx: &mut App| {
        tracing::info!(
            version = env!("CARGO_PKG_VERSION"),
            os = std::env::consts::OS,
            architecture = std::env::consts::ARCH,
            operation = "application.start",
            "starting Magenta"
        );
        cx.set_app_identity("magenta-1", "Magenta");
        gpui_component::init(cx);
        let mut startup_warnings = diagnostics_error.into_iter().collect::<Vec<_>>();

        if let Some(error) = magenta_ui::theme::init(cx).into_error() {
            tracing::warn!(
                error = ?error,
                code = error.presentation().code,
                operation = "theme.initialize",
                "using the default dark theme"
            );
            startup_warnings.push(error);
        }

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
                launch_failed_in_app.set(true);
                cx.quit();
            }
        }
    });

    drop(diagnostics_guard);
    if launch_failed.get() {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

fn open_main_window(cx: &mut App) -> Result<WindowHandle<Root>> {
    let bounds = Bounds::centered(None, size(px(1180.), px(760.)), cx);
    let mut window_options = gpui_component::TitleBar::window_options();
    window_options.window_bounds = Some(WindowBounds::Windowed(bounds));
    window_options.app_id = Some("magenta-1".into());
    if let Some(titlebar) = window_options.titlebar.as_mut() {
        titlebar.title = Some("Magenta".into());
    } else {
        tracing::warn!(
            operation = "window.configure",
            "GPUI did not provide titlebar options; using platform defaults"
        );
    }
    #[cfg(target_os = "linux")]
    {
        window_options.window_decorations = Some(WindowDecorations::Client);
        window_options.window_background = WindowBackgroundAppearance::Transparent;
    }

    cx.open_window(window_options, |window, cx| {
        let main_view = cx.new(|_| MainView::new());
        cx.new(|cx| Root::new(main_view, window, cx))
    })
    .map_err(|source| MagentaError::WindowOpen { source })
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
