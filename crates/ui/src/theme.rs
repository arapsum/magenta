use gpui::{App, SharedString};
use gpui_component::{Theme, ThemeConfig, ThemeMode, ThemeRegistry};
use std::rc::Rc;

use crate::{MagentaError, Result};

const BUILT_IN_THEMES: &str = include_str!("../../../themes/magenta.json");

pub const MAGENTA_LIGHT_THEME: &str = "Magenta Light";
pub const MAGENTA_DARK_THEME: &str = "Magenta Dark";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuiltInTheme {
    Light,
    Dark,
}

impl BuiltInTheme {
    #[must_use]
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

#[derive(Debug)]
pub enum ThemeInitOutcome {
    Applied(BuiltInTheme),
    Fallback {
        requested: BuiltInTheme,
        error: MagentaError,
    },
}

impl ThemeInitOutcome {
    #[must_use]
    pub const fn error(&self) -> Option<&MagentaError> {
        match self {
            Self::Applied(_) => None,
            Self::Fallback { error, .. } => Some(error),
        }
    }

    #[must_use]
    pub fn into_error(self) -> Option<MagentaError> {
        match self {
            Self::Applied(_) => None,
            Self::Fallback { error, .. } => Some(error),
        }
    }
}

/// Registers Magenta's bundled themes with gpui-component and applies the
/// dark appearance used by the product reference.
pub fn init(cx: &mut App) -> ThemeInitOutcome {
    init_from_str(BUILT_IN_THEMES, cx)
}

fn init_from_str(themes: &str, cx: &mut App) -> ThemeInitOutcome {
    let requested = BuiltInTheme::Dark;

    match load_and_apply_theme(themes, requested, cx) {
        Ok(()) => ThemeInitOutcome::Applied(requested),
        Err(error) => fallback_to_default_dark(requested, error, cx),
    }
}

fn load_and_apply_theme(themes: &str, requested: BuiltInTheme, cx: &mut App) -> Result<()> {
    ThemeRegistry::global_mut(cx)
        .load_themes_from_str(themes)
        .map_err(|source| MagentaError::ThemeLoad { source })?;
    apply(requested, cx)
}

fn fallback_to_default_dark(
    requested: BuiltInTheme,
    error: MagentaError,
    cx: &mut App,
) -> ThemeInitOutcome {
    apply_default_dark(cx);
    ThemeInitOutcome::Fallback { requested, error }
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

/// Applies one of Magenta's bundled themes.
///
/// # Errors
///
/// Returns [`MagentaError::ThemeNotFound`] when the bundled theme has not
/// been registered.
pub fn apply(theme: BuiltInTheme, cx: &mut App) -> Result<()> {
    apply_named(theme.name(), cx)
}

/// Applies any registered gpui-component theme by name. A future theme picker
/// or persisted preference can use the same entry point.
///
/// # Errors
///
/// Returns [`MagentaError::ThemeNotFound`] when no registered theme has the
/// requested name.
pub fn apply_named(name: &str, cx: &mut App) -> Result<()> {
    let name = SharedString::from(name);
    let theme = ThemeRegistry::global(cx)
        .themes()
        .get(&name)
        .cloned()
        .ok_or_else(|| MagentaError::ThemeNotFound {
            name: name.to_string(),
        })?;

    apply_config(theme, cx);
    Ok(())
}

/// Switches between Magenta's bundled light and dark themes.
///
/// # Errors
///
/// Returns [`MagentaError::ThemeNotFound`] when the target bundled theme has
/// not been registered.
pub fn toggle(cx: &mut App) -> Result<BuiltInTheme> {
    let next = if Theme::global(cx).is_dark() {
        BuiltInTheme::Light
    } else {
        BuiltInTheme::Dark
    };
    apply(next, cx)?;
    Ok(next)
}

fn apply_default_dark(cx: &mut App) {
    Theme::change(ThemeMode::Dark, None, cx);
    cx.refresh_windows();
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

#[cfg(test)]
mod tests {
    use gpui::TestAppContext;

    use super::*;

    #[gpui::test]
    fn unknown_theme_is_reported_without_changing_the_active_theme(cx: &TestAppContext) {
        cx.update(|cx| {
            gpui_component::init(cx);
            let outcome = init(cx);
            assert!(matches!(
                outcome,
                ThemeInitOutcome::Applied(BuiltInTheme::Dark)
            ));

            let before = Theme::global(cx).mode;
            let error = apply_named("Theme That Does Not Exist", cx).unwrap_err();

            assert!(matches!(error, MagentaError::ThemeNotFound { .. }));
            assert_eq!(Theme::global(cx).mode, before);
        });
    }

    #[gpui::test]
    fn bundled_themes_toggle_successfully(cx: &TestAppContext) {
        cx.update(|cx| {
            gpui_component::init(cx);
            assert!(matches!(
                init(cx),
                ThemeInitOutcome::Applied(BuiltInTheme::Dark)
            ));
            assert!(Theme::global(cx).is_dark());

            assert_eq!(toggle(cx).unwrap(), BuiltInTheme::Light);
            assert!(!Theme::global(cx).is_dark());
            assert_eq!(toggle(cx).unwrap(), BuiltInTheme::Dark);
            assert!(Theme::global(cx).is_dark());
        });
    }

    #[gpui::test]
    fn malformed_bundled_theme_falls_back_to_default_dark(cx: &TestAppContext) {
        cx.update(|cx| {
            gpui_component::init(cx);

            let outcome = init_from_str("{ definitely not valid json", cx);

            assert!(matches!(
                outcome,
                ThemeInitOutcome::Fallback {
                    requested: BuiltInTheme::Dark,
                    error: MagentaError::ThemeLoad { .. }
                }
            ));
            assert!(Theme::global(cx).is_dark());
        });
    }

    #[gpui::test]
    fn missing_magenta_dark_falls_back_without_panicking(cx: &TestAppContext) {
        cx.update(|cx| {
            gpui_component::init(cx);

            let outcome = init_from_str(r#"{"name":"Empty","themes":[]}"#, cx);

            assert!(matches!(
                outcome,
                ThemeInitOutcome::Fallback {
                    requested: BuiltInTheme::Dark,
                    error: MagentaError::ThemeNotFound { .. }
                }
            ));
            assert!(Theme::global(cx).is_dark());
        });
    }
}
