use std::error::Error as _;

use gpui_kit::component::Root;
use gpui_kit::{
    AppContext as _, Context, IntoElement, Render, TestAppContext, Window, div, px, size,
};

use super::*;

struct EmptyView;

impl Render for EmptyView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<'_, Self>) -> impl IntoElement {
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
        MagentaError::SendMessage {
            source: SendMessageError::EmptyPrompt,
        },
        MagentaError::RegenerateMessage {
            source: RegenerateMessageError::Storage(magenta_core::StorageError::new(
                magenta_core::StorageErrorKind::NotFound,
                std::io::Error::other("missing response"),
            )),
        },
        MagentaError::RetryMessage {
            source: RetryMessageError::AgentContinuation,
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

#[gpui_kit::test]
fn repeated_errors_replace_their_existing_notification(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
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
