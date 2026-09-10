//! Application workflows that coordinate Magenta's domain values and ports.

mod agent;
mod error;
mod history;
mod projects;
mod regenerate_message;
mod send_message;

pub use agent::{
    AgentApprovalController, AgentSendTarget, PendingAgentGeneration, RetryWorkspaceAgentInput,
    RunWorkspaceAgent, RunWorkspaceAgentInput,
};
pub use error::{
    RegenerateMessageError, RetryMessageError, SendMessageError, TitleConversationError,
};
pub use history::ConversationHistory;
pub use projects::{ProjectCatalog, ProjectCatalogError};
pub use regenerate_message::{
    PendingRegeneration, PendingRetry, RegenerateMessage, RegenerateMessageInput, RetryMessageInput,
};
pub use send_message::{PendingGeneration, SendMessage, SendMessageInput, SendTarget};
