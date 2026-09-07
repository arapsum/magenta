//! Application workflows that coordinate Magenta's domain values and ports.

mod agent;
mod error;
mod history;
mod regenerate_message;
mod send_message;

pub use agent::{
    AgentApprovalController, AgentSendTarget, PendingAgentGeneration, RunWorkspaceAgent,
    RunWorkspaceAgentInput,
};
pub use error::{RegenerateMessageError, SendMessageError};
pub use history::ConversationHistory;
pub use regenerate_message::{PendingRegeneration, RegenerateMessage, RegenerateMessageInput};
pub use send_message::{PendingGeneration, SendMessage, SendMessageInput, SendTarget};
