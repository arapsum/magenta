use std::{f32::consts::TAU, time::Duration};

use gpui_kit::component::{ActiveTheme as _, box_shadow};
use gpui_kit::{
    Animation, AnimationExt as _, AnyElement, App, Hsla, IntoElement as _, ObjectFit,
    ParentElement as _, Styled as _, StyledImage as _, img,
};

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

pub fn ambient_field(cx: &App) -> AnyElement {
    if !cx.theme().is_dark() {
        return gpui_kit::div().absolute().inset_0().into_any_element();
    }

    gpui_kit::div()
        .absolute()
        .inset_0()
        .overflow_hidden()
        .child(
            img("icons/surface-glow.svg")
                .absolute()
                .inset_0()
                .size_full()
                .object_fit(ObjectFit::Cover)
                .with_animation(
                    "ambient-field",
                    Animation::new(Duration::from_secs(28))
                        .repeat_synced()
                        .with_max_fps(20.),
                    |field, delta| {
                        let breath = (delta * TAU).sin().mul_add(0.035, 0.54);
                        field.opacity(breath)
                    },
                ),
        )
        .into_any_element()
}
