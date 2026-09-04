use gpui::{px, size, App, AppContext, Application, Bounds, WindowBounds};
#[cfg(target_os = "linux")]
use gpui::{WindowBackgroundAppearance, WindowDecorations};

use magenta_ui::MainView;

fn main() {
    let app: Application = gpui_platform::application().with_assets(gpui_component_assets::Assets);

    app.run(|cx: &mut App| {
        cx.set_app_identity("magenta-1", "Magenta");
        gpui_component::init(cx);
        magenta_ui::theme::init(cx);

        let bounds = Bounds::centered(None, size(px(1180.), px(760.)), cx);
        let mut window_options = gpui_component::TitleBar::window_options();
        window_options.window_bounds = Some(WindowBounds::Windowed(bounds));
        window_options.app_id = Some("magenta-1".into());
        window_options.titlebar.as_mut().unwrap().title = Some("Magenta".into());
        #[cfg(target_os = "linux")]
        {
            window_options.window_decorations = Some(WindowDecorations::Client);
            window_options.window_background = WindowBackgroundAppearance::Transparent;
        }

        cx.open_window(window_options, |window, cx| {
            let main_view = cx.new(|_| MainView::new());
            cx.new(|cx| gpui_component::Root::new(main_view, window, cx))
        })
        .unwrap();
    });
}
