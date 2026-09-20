//! Application workflows that coordinate Magenta's domain values and ports.
//!
//! [`SendMessage`], [`RegenerateMessage`], and [`RunWorkspaceAgent`] prepare
//! durable turns before returning provider streams. Callers own consumption,
//! cancellation, and terminal-message saving through [`ConversationHistory`].
//! [`ProjectCatalog`] registers workspaces and exposes read-only browsing.
//! Runtime dependencies use core ports rather than concrete adapters or GPUI.

mod agent;
mod commands;
mod error;
mod generation_defaults;
mod history;
mod projects;
mod regenerate_message;
mod send_message;
pub mod trace;

pub use agent::{
    AgentApprovalController, AgentSendTarget, PendingAgentGeneration, RetryWorkspaceAgentInput,
    RunWorkspaceAgent, RunWorkspaceAgentInput,
};
pub use commands::{
    CommandResolution, CommandResolutionError, apply_provider_prompt, resolve_normal,
    resolve_persisted, resolve_submission,
};
pub use error::{
    RegenerateMessageError, RetryMessageError, SendMessageError, TitleConversationError,
};
pub use generation_defaults::{
    GenerationFallbackReason, GenerationResolution, resolve_generation_defaults,
};
pub use history::ConversationHistory;
pub use projects::{ProjectCatalog, ProjectCatalogError};
pub use regenerate_message::{
    PendingRegeneration, PendingRetry, RegenerateMessage, RegenerateMessageInput, RetryMessageInput,
};
pub use send_message::{PendingGeneration, SendMessage, SendMessageInput, SendTarget};
