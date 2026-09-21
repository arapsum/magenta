use gpui_kit::component::{ActiveTheme as _, box_shadow};
use gpui_kit::{App, Hsla};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SurfaceLevel {
    Recessed,
    Raised,
    Floating,
}

pub fn surface(level: SurfaceLevel, cx: &App) -> Hsla {
    if !cx.theme().is_dark() {
        return match level {
            SurfaceLevel::Recessed => cx.theme().muted,
            SurfaceLevel::Raised | SurfaceLevel::Floating => cx.theme().popover,
        };
    }

    match level {
        SurfaceLevel::Recessed => cx.theme().background,
        SurfaceLevel::Raised => cx.theme().secondary,
        SurfaceLevel::Floating => cx.theme().popover,
    }
}

pub fn floating_shadow(cx: &App) -> Vec<gpui_kit::BoxShadow> {
    vec![
        box_shadow(0., 22., 56., -28., cx.theme().background.opacity(0.98)),
        box_shadow(0., 4., 14., -8., cx.theme().border.opacity(0.42)),
    ]
}
