use gpui::{App, Global, SharedString, px};
use gpui_component::{Theme, ThemeMode};
use magenta_core::{AppSettings, AppearanceMode, FontChoice, MathFontStyle};

use crate::theme::{self, BuiltInTheme};

/// Process-wide settings that affect every Magenta window.
pub struct SettingsGlobal(pub AppSettings);

impl Global for SettingsGlobal {}

pub fn init(cx: &mut App) {
    cx.set_global(SettingsGlobal(AppSettings::default()));
}

pub fn current(cx: &App) -> AppSettings {
    cx.global::<SettingsGlobal>().0.clone()
}

pub fn replace(settings: AppSettings, cx: &mut App) {
    apply_appearance(&settings, cx);
    cx.global_mut::<SettingsGlobal>().0 = settings;
    cx.refresh_windows();
}

pub fn update(mutator: impl FnOnce(&mut AppSettings), cx: &mut App) -> AppSettings {
    let mut settings = current(cx);
    mutator(&mut settings);
    replace(settings.clone(), cx);
    settings
}

pub fn math_style(cx: &App) -> MathFontStyle {
    current(cx).typography.math_font
}

pub fn inline_math_size(cx: &App) -> u16 {
    current(cx).typography.inline_math_size
}

pub fn display_math_size(cx: &App) -> u16 {
    current(cx).typography.display_math_size
}

fn apply_appearance(settings: &AppSettings, cx: &mut App) {
    let theme = match settings.appearance {
        AppearanceMode::System => BuiltInTheme::from(cx.window_appearance()),
        AppearanceMode::Light => BuiltInTheme::Light,
        AppearanceMode::Dark => BuiltInTheme::Dark,
    };
    if let Err(error) = theme::apply(theme, cx) {
        tracing::warn!(?error, "could not apply configured appearance");
    }

    let active = Theme::global_mut(cx);
    active.font_family = resolve_font(&settings.typography.ui_font);
    active.font_size = px(f32::from(settings.typography.ui_size));
    active.mono_font_family = resolve_font(&settings.typography.monospace_font);
    active.mono_font_size = px(f32::from(settings.typography.monospace_size));
    Theme::sync_base(cx);
}

fn resolve_font(choice: &FontChoice) -> SharedString {
    match choice {
        FontChoice::SystemUi => ".SystemUIFont".into(),
        FontChoice::SystemMonospace => {
            if cfg!(target_os = "macos") {
                "Menlo".into()
            } else if cfg!(target_os = "windows") {
                "Consolas".into()
            } else {
                "DejaVu Sans Mono".into()
            }
        }
        FontChoice::Family(name) => name.clone().into(),
    }
}

impl From<gpui::WindowAppearance> for BuiltInTheme {
    fn from(appearance: gpui::WindowAppearance) -> Self {
        if ThemeMode::from(appearance).is_dark() {
            Self::Dark
        } else {
            Self::Light
        }
    }
}
