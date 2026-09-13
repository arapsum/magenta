use gpui_kit::TestAppContext;

use super::*;

#[gpui_kit::test]
fn unknown_theme_is_reported_without_changing_the_active_theme(cx: &TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
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

#[gpui_kit::test]
fn bundled_themes_toggle_successfully(cx: &TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
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

#[gpui_kit::test]
fn malformed_bundled_theme_falls_back_to_default_dark(cx: &TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);

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

#[gpui_kit::test]
fn missing_magenta_dark_falls_back_without_panicking(cx: &TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);

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
