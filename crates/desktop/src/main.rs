use gpui::{px, size, App, AppContext, Application, Bounds, QuitMode, WindowBounds, WindowOptions};

use magenta_ui::Magenta;

fn main() {
    let app: Application = gpui_platform::application().with_quit_mode(QuitMode::Explicit);

    app.run(|cx: &mut App| {
        cx.set_app_identity("magenta-1", "Magenta");

        let bounds = Bounds::centered(None, size(px(500.0), px(500.)), cx);

        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |_, cx| {
                cx.new(|_| Magenta {
                    text: "World".into(),
                })
            },
        )
        .unwrap();
    });
}
