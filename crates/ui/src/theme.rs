use gpui::{App, SharedString};
use gpui_component::{Theme, ThemeConfig, ThemeMode, ThemeRegistry};
use std::rc::Rc;

const BUILT_IN_THEMES: &str = include_str!("../../../themes/magenta.json");

pub const MAGENTA_LIGHT_THEME: &str = "Magenta Light";
pub const MAGENTA_DARK_THEME: &str = "Magenta Dark";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuiltInTheme {
    Light,
    Dark,
}

impl BuiltInTheme {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Light => MAGENTA_LIGHT_THEME,
            Self::Dark => MAGENTA_DARK_THEME,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThemeOption {
    pub name: SharedString,
    pub mode: ThemeMode,
}

/// Registers Magenta's bundled themes with gpui-component and applies the
/// dark appearance used by the product reference.
pub fn init(cx: &mut App) {
    ThemeRegistry::global_mut(cx)
        .load_themes_from_str(BUILT_IN_THEMES)
        .expect("Magenta's bundled theme JSON must be valid");

    assert!(
        apply(BuiltInTheme::Dark, cx),
        "Magenta Dark must be present in the bundled theme set"
    );
}

/// Returns every theme known to gpui-component, including themes registered
/// by the application later.
pub fn available(cx: &App) -> Vec<ThemeOption> {
    ThemeRegistry::global(cx)
        .sorted_themes()
        .into_iter()
        .map(|theme| ThemeOption {
            name: theme.name.clone(),
            mode: theme.mode,
        })
        .collect()
}

pub fn apply(theme: BuiltInTheme, cx: &mut App) -> bool {
    apply_named(theme.name(), cx)
}

/// Applies any registered gpui-component theme by name. A future theme picker
/// or persisted preference can use the same entry point.
pub fn apply_named(name: &str, cx: &mut App) -> bool {
    let name = SharedString::from(name);
    let Some(theme) = ThemeRegistry::global(cx).themes().get(&name).cloned() else {
        return false;
    };

    apply_config(theme, cx);
    true
}

pub fn toggle(cx: &mut App) -> BuiltInTheme {
    let next = if Theme::global(cx).is_dark() {
        BuiltInTheme::Light
    } else {
        BuiltInTheme::Dark
    };
    let _ = apply(next, cx);
    next
}

fn apply_config(theme: Rc<ThemeConfig>, cx: &mut App) {
    let mode = theme.mode;
    let active_theme = Theme::global_mut(cx);

    if mode.is_dark() {
        active_theme.dark_theme = theme;
    } else {
        active_theme.light_theme = theme;
    }

    Theme::change(mode, None, cx);
    cx.refresh_windows();
}
