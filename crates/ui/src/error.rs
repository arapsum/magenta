use std::path::PathBuf;

use gpui_kit::component::notification::{Notification, NotificationType};
use magenta_application::{RegenerateMessageError, RetryMessageError, SendMessageError};
use magenta_core::{ProviderError, ProviderId};

/// The errors that can cross Magenta's subsystem boundaries.
///
/// Each variant preserves its technical source for diagnostics. User-facing
/// copy is intentionally provided by [`MagentaError::presentation`] instead of
/// exposing raw error messages in the interface.
#[derive(Debug, thiserror::Error)]
pub enum MagentaError {
    #[error("failed to initialize conversation storage")]
    StorageInitialize {
        #[source]
        source: magenta_core::StorageError,
    },
    #[error("failed to load conversation history")]
    StorageLoad {
        #[source]
        source: magenta_core::StorageError,
    },
    #[error("failed to save conversation history")]
    StorageWrite {
        #[source]
        source: magenta_core::StorageError,
    },
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

    #[error("send-message workflow failed")]
    SendMessage {
        #[source]
        source: SendMessageError,
    },

    #[error("regenerate-message workflow failed")]
    RegenerateMessage {
        #[source]
        source: RegenerateMessageError,
    },

    #[error("retry-message workflow failed")]
    RetryMessage {
        #[source]
        source: RetryMessageError,
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
    #[allow(clippy::too_many_lines)]
    pub const fn presentation(&self) -> ErrorPresentation {
        match self {
            Self::StorageInitialize { .. } => ErrorPresentation {
                code: "MAG-STORAGE-INIT",
                severity: ErrorSeverity::Error,
                title: "History unavailable",
                message: concat!(
                    "Magenta could not open local history. ",
                    "Retry from the sidebar before sending messages."
                ),
            },
            Self::StorageLoad { .. } => ErrorPresentation {
                code: "MAG-STORAGE-LOAD",
                severity: ErrorSeverity::Error,
                title: "History could not be loaded",
                message: "Your current conversation was kept. Try opening the conversation again.",
            },
            Self::StorageWrite { .. } => ErrorPresentation {
                code: "MAG-STORAGE-WRITE",
                severity: ErrorSeverity::Error,
                title: "Changes could not be saved",
                message: concat!(
                    "Your response is still visible. ",
                    "Retry saving before leaving this conversation."
                ),
            },
            Self::ThemeLoad { .. } => ErrorPresentation {
                code: "MAG-THEME-LOAD",
                severity: ErrorSeverity::Warning,
                title: "Theme fallback enabled",
                message: concat!(
                    "Magenta could not load its bundled theme, ",
                    "so the default dark theme is in use."
                ),
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
                message: concat!(
                    "Magenta could not create its local log file. ",
                    "Diagnostics will be written to the console instead."
                ),
            },
            Self::AttachmentPicker { .. } => ErrorPresentation {
                code: "MAG-ATTACHMENT-PICKER",
                severity: ErrorSeverity::Warning,
                title: "Images could not be selected",
                message: concat!(
                    "The system image picker could not be opened. ",
                    "Try adding the reference images again."
                ),
            },
            Self::ProviderGeneration { .. } => ErrorPresentation {
                code: "MAG-PROVIDER-GENERATION",
                severity: ErrorSeverity::Error,
                title: "Response could not be generated",
                message: concat!(
                    "The selected model could not finish the response. ",
                    "Try again or choose another model."
                ),
            },
            Self::SendMessage { source } => send_message_presentation(source),
            Self::RegenerateMessage { source } => match source {
                RegenerateMessageError::Storage(error) => match error.kind {
                    magenta_core::StorageErrorKind::ContextTooLarge => {
                        context_too_large_presentation()
                    }
                    _ => ErrorPresentation {
                        code: "MAG-REGENERATE-MESSAGE",
                        severity: ErrorSeverity::Error,
                        title: "Response could not be regenerated",
                        message: concat!(
                            "Magenta could not prepare this response. ",
                            "Try another completed response."
                        ),
                    },
                },
            },
            Self::RetryMessage { source } => match source {
                RetryMessageError::Storage(error) => match error.kind {
                    magenta_core::StorageErrorKind::ContextTooLarge => {
                        context_too_large_presentation()
                    }
                    _ => ErrorPresentation {
                        code: "MAG-RETRY-MESSAGE",
                        severity: ErrorSeverity::Error,
                        title: "Response could not be retried",
                        message: "Magenta kept the failed response. Try again or prepare a continuation.",
                    },
                },
                RetryMessageError::AgentContinuation => ErrorPresentation {
                    code: "MAG-RETRY-AGENT-CONTINUATION",
                    severity: ErrorSeverity::Warning,
                    title: "Continue from the completed work",
                    message: "This agent response may have changed files. Review the workspace, then continue manually.",
                },
                RetryMessageError::WorkspaceUnavailable => ErrorPresentation {
                    code: "MAG-RETRY-AGENT-WORKSPACE",
                    severity: ErrorSeverity::Warning,
                    title: "Workspace unavailable",
                    message: "Choose the conversation workspace again, then prepare a continuation.",
                },
                RetryMessageError::WorkspaceSession(_) => ErrorPresentation {
                    code: "MAG-RETRY-AGENT-SESSION",
                    severity: ErrorSeverity::Warning,
                    title: "Workspace is busy",
                    message: "Finish the current Work run, or apply or discard its staged changes, before retrying.",
                },
            },
        }
    }
}

/// Maps provider failures to safe account/model-settings copy.
#[must_use]
pub const fn provider_error_presentation(
    kind: magenta_core::ProviderErrorKind,
) -> ErrorPresentation {
    match kind {
        magenta_core::ProviderErrorKind::AuthenticationRequired => ErrorPresentation {
            code: "MAG-ACCOUNT-AUTH",
            severity: ErrorSeverity::Error,
            title: "Sign-in required",
            message: "Reconnect the provider account to load models and generate responses.",
        },
        magenta_core::ProviderErrorKind::PermissionDenied => ErrorPresentation {
            code: "MAG-ACCOUNT-PERMISSION",
            severity: ErrorSeverity::Error,
            title: "Account access denied",
            message: "This account cannot access the requested provider resource.",
        },
        magenta_core::ProviderErrorKind::RateLimited => ErrorPresentation {
            code: "MAG-ACCOUNT-RATE-LIMIT",
            severity: ErrorSeverity::Warning,
            title: "Provider is temporarily busy",
            message: "Wait a moment, then reload the account or models.",
        },
        magenta_core::ProviderErrorKind::Transport => ErrorPresentation {
            code: "MAG-ACCOUNT-CONNECTION",
            severity: ErrorSeverity::Error,
            title: "Provider connection failed",
            message: "Check the connection and retry loading the account.",
        },
        magenta_core::ProviderErrorKind::ServiceUnavailable => ErrorPresentation {
            code: "MAG-ACCOUNT-SERVICE",
            severity: ErrorSeverity::Error,
            title: "Provider unavailable",
            message: "The provider is temporarily unavailable. Retry in a moment.",
        },
        _ => ErrorPresentation {
            code: "MAG-ACCOUNT-LOAD",
            severity: ErrorSeverity::Error,
            title: "Provider setup unavailable",
            message: "Magenta could not finish loading this provider. Retry from settings.",
        },
    }
}

const fn send_message_presentation(source: &SendMessageError) -> ErrorPresentation {
    use magenta_core::StorageErrorKind;

    match source {
        SendMessageError::EmptyPrompt => ErrorPresentation {
            code: "MAG-SEND-EMPTY",
            severity: ErrorSeverity::Warning,
            title: "Add a message or image",
            message: "Write a message or attach an image before sending.",
        },
        SendMessageError::WorkspaceUnavailable => ErrorPresentation {
            code: "MAG-WORKSPACE-UNAVAILABLE",
            severity: ErrorSeverity::Warning,
            title: "Workspace unavailable",
            message: "Choose an existing workspace directory before starting agent mode.",
        },
        SendMessageError::WorkspaceSession(_) => ErrorPresentation {
            code: "MAG-WORKSPACE-SESSION",
            severity: ErrorSeverity::Warning,
            title: "Workspace is busy",
            message: "Finish the current Work run, or apply or discard its staged changes, before sending another Work message.",
        },
        SendMessageError::Storage(error) => match error.kind {
            StorageErrorKind::TooManyAttachments => ErrorPresentation {
                code: "MAG-ATTACHMENT-COUNT",
                severity: ErrorSeverity::Warning,
                title: "Too many images",
                message: "Attach up to four images in one message.",
            },
            StorageErrorKind::AttachmentUnreadable => ErrorPresentation {
                code: "MAG-ATTACHMENT-READ",
                severity: ErrorSeverity::Warning,
                title: "Image unavailable",
                message: "Magenta could not read one of the selected images.",
            },
            StorageErrorKind::UnsupportedAttachment => ErrorPresentation {
                code: "MAG-ATTACHMENT-TYPE",
                severity: ErrorSeverity::Warning,
                title: "Unsupported image",
                message: "Use a PNG, JPEG, WebP, or non-animated GIF image.",
            },
            StorageErrorKind::AnimatedImage => ErrorPresentation {
                code: "MAG-ATTACHMENT-ANIMATED",
                severity: ErrorSeverity::Warning,
                title: "Animated GIF not supported",
                message: "Attach a still image instead.",
            },
            StorageErrorKind::AttachmentTooLarge => ErrorPresentation {
                code: "MAG-ATTACHMENT-SIZE",
                severity: ErrorSeverity::Warning,
                title: "Image is too large",
                message: "Each image must be 10 MiB or smaller.",
            },
            StorageErrorKind::ContextTooLarge => context_too_large_presentation(),
            _ => ErrorPresentation {
                code: "MAG-SEND-MESSAGE",
                severity: ErrorSeverity::Error,
                title: "Message could not be sent",
                message: "Magenta could not save this message. Try again.",
            },
        },
    }
}

const fn context_too_large_presentation() -> ErrorPresentation {
    ErrorPresentation {
        code: "MAG-CONTEXT-TOO-LARGE",
        severity: ErrorSeverity::Warning,
        title: "Context limit reached",
        message: "Shorten this message or remove some images and context, then try again.",
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
        .autohide(presentation.severity == ErrorSeverity::Warning)
}

#[cfg(test)]
#[path = "../test/error.rs"]
mod tests;
