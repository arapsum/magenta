use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::{App, Global, SharedString, px};
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

    let ui_font = resolve_font(&settings.typography.ui_font, cx);
    let monospace_font = resolve_font(&settings.typography.monospace_font, cx);
    let active = Theme::global_mut(cx);
    active.font_family = ui_font;
    active.font_size = px(f32::from(settings.typography.ui_size));
    active.mono_font_family = monospace_font;
    active.mono_font_size = px(f32::from(settings.typography.monospace_size));
    Theme::sync_base(cx);
}

fn resolve_font(choice: &FontChoice, cx: &App) -> SharedString {
    match choice {
        FontChoice::SystemUi => preferred_installed_font(
            cx,
            &[
                "Manrope",
                "Avenir Next",
                "Segoe UI Variable",
                "Segoe UI",
                "Adwaita Sans",
                "Noto Sans",
            ],
            ".SystemUIFont",
        ),
        FontChoice::SystemMonospace => preferred_installed_font(
            cx,
            &[
                "Google Sans Code",
                "JetBrains Mono",
                "SF Mono",
                "Cascadia Mono",
                "IBM Plex Mono",
                "Menlo",
                "Consolas",
                "Noto Sans Mono",
                "DejaVu Sans Mono",
            ],
            ".SystemUIFont",
        ),
        FontChoice::Family(name) => name.clone().into(),
    }
}

fn preferred_installed_font(cx: &App, candidates: &[&str], fallback: &str) -> SharedString {
    let installed = cx.text_system().all_font_names();
    candidates
        .iter()
        .find(|candidate| {
            installed
                .iter()
                .any(|font| font.eq_ignore_ascii_case(candidate))
        })
        .map_or_else(|| fallback.into(), |font| (*font).into())
}

impl From<gpui_kit::WindowAppearance> for BuiltInTheme {
    fn from(appearance: gpui_kit::WindowAppearance) -> Self {
        if ThemeMode::from(appearance).is_dark() {
            Self::Dark
        } else {
            Self::Light
        }
    }
}
