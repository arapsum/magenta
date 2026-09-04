use std::path::PathBuf;

use gpui_component::notification::{Notification, NotificationType};
use magenta_core::{ProviderError, ProviderId};

/// The errors that can cross Magenta's subsystem boundaries.
///
/// Each variant preserves its technical source for diagnostics. User-facing
/// copy is intentionally provided by [`MagentaError::presentation`] instead of
/// exposing raw error messages in the interface.
#[derive(Debug, thiserror::Error)]
pub enum MagentaError {
    #[error("failed to load theme definitions")]
    ThemeLoad {
        #[source]
        source: anyhow::Error,
    },

    #[error("theme `{name}` was not found")]
    ThemeNotFound { name: String },

    #[error("failed to open the application window")]
    WindowOpen {
        #[source]
        source: anyhow::Error,
    },

    #[error("failed to initialize diagnostics at `{path}`")]
    Diagnostics {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to open the reference image picker")]
    AttachmentPicker {
        #[source]
        source: anyhow::Error,
    },

    #[error("provider generation failed")]
    ProviderGeneration {
        provider: ProviderId,
        #[source]
        source: ProviderError,
    },
}

pub type Result<T> = std::result::Result<T, MagentaError>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorSeverity {
    Warning,
    Error,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ErrorPresentation {
    pub code: &'static str,
    pub severity: ErrorSeverity,
    pub title: &'static str,
    pub message: &'static str,
}

impl MagentaError {
    /// Returns stable, privacy-safe copy suitable for display to the user.
    #[must_use]
    pub const fn presentation(&self) -> ErrorPresentation {
        match self {
            Self::ThemeLoad { .. } => ErrorPresentation {
                code: "MAG-THEME-LOAD",
                severity: ErrorSeverity::Warning,
                title: "Theme fallback enabled",
                message: "Magenta could not load its bundled theme, so the default dark theme is in use.",
            },
            Self::ThemeNotFound { .. } => ErrorPresentation {
                code: "MAG-THEME-NOT-FOUND",
                severity: ErrorSeverity::Error,
                title: "Theme unavailable",
                message: "The selected theme is no longer available. Your current theme was kept.",
            },
            Self::WindowOpen { .. } => ErrorPresentation {
                code: "MAG-WINDOW-OPEN",
                severity: ErrorSeverity::Error,
                title: "Magenta could not open",
                message: "The main window could not be created. Restart Magenta and try again.",
            },
            Self::Diagnostics { .. } => ErrorPresentation {
                code: "MAG-DIAGNOSTICS",
                severity: ErrorSeverity::Warning,
                title: "Local diagnostics unavailable",
                message: "Magenta could not create its local log file. Diagnostics will be written to the console instead.",
            },
            Self::AttachmentPicker { .. } => ErrorPresentation {
                code: "MAG-ATTACHMENT-PICKER",
                severity: ErrorSeverity::Warning,
                title: "Images could not be selected",
                message: "The system image picker could not be opened. Try adding the reference images again.",
            },
            Self::ProviderGeneration { .. } => ErrorPresentation {
                code: "MAG-PROVIDER-GENERATION",
                severity: ErrorSeverity::Error,
                title: "Response could not be generated",
                message: "The selected model could not finish the response. Try again or choose another model.",
            },
        }
    }
}

struct ErrorNotification;

/// Builds a persistent, deduplicated notification from a typed application
/// error. Raw sources and local paths are never included in the notification.
#[must_use]
pub fn notification_for_error(error: &MagentaError) -> Notification {
    let presentation = error.presentation();
    let notification_type = match presentation.severity {
        ErrorSeverity::Warning => NotificationType::Warning,
        ErrorSeverity::Error => NotificationType::Error,
    };

    Notification::new()
        .id1::<ErrorNotification>(presentation.code)
        .title(presentation.title)
        .message(format!(
            "{} Reference: {}",
            presentation.message, presentation.code
        ))
        .with_type(notification_type)
        .autohide(false)
}

#[cfg(test)]
mod tests {
    use std::error::Error as _;

    use gpui::{
        AppContext as _, Context, IntoElement, Render, TestAppContext, Window, div, px, size,
    };
    use gpui_component::Root;

    use super::*;

    struct EmptyView;

    impl Render for EmptyView {
        fn render(
            &mut self,
            _window: &mut Window,
            _cx: &mut Context<'_, Self>,
        ) -> impl IntoElement {
            div()
        }
    }

    #[test]
    fn user_presentations_do_not_expose_sources_or_paths() {
        let secret_path = PathBuf::from("/home/example/private/logs");
        let error = MagentaError::Diagnostics {
            path: secret_path.clone(),
            source: std::io::Error::new(std::io::ErrorKind::PermissionDenied, "token=secret"),
        };

        let presentation = error.presentation();
        assert_eq!(presentation.code, "MAG-DIAGNOSTICS");
        assert!(!presentation.message.contains("token=secret"));
        assert!(
            !presentation
                .message
                .contains(&secret_path.display().to_string())
        );
    }

    #[test]
    fn technical_errors_preserve_their_source_chain() {
        let error = MagentaError::ThemeLoad {
            source: anyhow::anyhow!("invalid JSON at line 4"),
        };

        assert_eq!(error.to_string(), "failed to load theme definitions");
        assert_eq!(
            error.source().map(ToString::to_string),
            Some("invalid JSON at line 4".to_owned())
        );
    }

    #[test]
    fn every_error_variant_has_a_stable_distinct_code() {
        let errors = [
            MagentaError::ThemeLoad {
                source: anyhow::anyhow!("parse"),
            },
            MagentaError::ThemeNotFound {
                name: "Missing".to_owned(),
            },
            MagentaError::WindowOpen {
                source: anyhow::anyhow!("platform"),
            },
            MagentaError::Diagnostics {
                path: PathBuf::from("logs"),
                source: std::io::Error::other("disk"),
            },
            MagentaError::AttachmentPicker {
                source: anyhow::anyhow!("portal unavailable"),
            },
            MagentaError::ProviderGeneration {
                provider: ProviderId::new("anthropic"),
                source: ProviderError::new(
                    ProviderId::new("anthropic"),
                    std::io::Error::other("connection closed"),
                ),
            },
        ];

        let mut codes = errors
            .iter()
            .map(|error| error.presentation().code)
            .collect::<Vec<_>>();
        codes.sort_unstable();
        codes.dedup();

        assert_eq!(codes.len(), errors.len());
    }

    #[gpui::test]
    fn repeated_errors_replace_their_existing_notification(cx: &mut TestAppContext) {
        cx.update(gpui_component::init);
        let window_handle = cx.open_window(size(px(480.), px(320.)), |window, cx| {
            let view = cx.new(|_| EmptyView);
            Root::new(view, window, cx)
        });

        window_handle
            .update(cx, |root, window, cx| {
                for _ in 0..2 {
                    let error = MagentaError::ThemeNotFound {
                        name: "Missing".to_owned(),
                    };
                    root.push_notification(notification_for_error(&error), window, cx);
                }

                assert_eq!(root.notification.read(cx).notifications().len(), 1);
            })
            .expect("the test window should remain open");
    }
}
