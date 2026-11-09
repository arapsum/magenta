use gpui_kit::component::{ActiveTheme as _, Icon, box_shadow};
use gpui_kit::{App, Hsla, IntoElement as _, ParentElement as _, Styled as _, div, px, rgba};

pub fn brand_mark(size: f32, cx: &App) -> gpui_kit::AnyElement {
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(size))
        .rounded(px(size * 0.3))
        .bg(cx.theme().accent)
        .text_color(cx.theme().magenta)
        .child(
            Icon::empty()
                .path("icons/magenta.svg")
                .size(px(size * 0.65)),
        )
        .into_any_element()
}

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
            SurfaceLevel::Raised => cx.theme().secondary,
            SurfaceLevel::Floating => cx.theme().popover,
        };
    }

    match level {
        SurfaceLevel::Recessed => cx.theme().background,
        SurfaceLevel::Raised => cx.theme().secondary,
        SurfaceLevel::Floating => cx.theme().popover,
    }
}

pub fn floating_shadow(cx: &App) -> Vec<gpui_kit::BoxShadow> {
    let dark = cx.theme().is_dark();
    vec![
        box_shadow(
            0.,
            12.,
            32.,
            -12.,
            rgba(0x0000_00ff)
                .opacity(if dark { 0.32 } else { 0.08 })
                .into(),
        ),
        box_shadow(
            0.,
            2.,
            6.,
            -2.,
            rgba(0x0000_00ff)
                .opacity(if dark { 0.24 } else { 0.04 })
                .into(),
        ),
    ]
}
