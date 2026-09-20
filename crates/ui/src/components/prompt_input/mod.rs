mod attachments;
mod commands;
mod render;
mod state;
#[cfg(test)]
#[path = "../../../test/components/prompt_input/mod.rs"]
mod tests;

pub use state::AgentCapability;
pub use state::{PromptComposer, PromptComposerEvent, PromptRequest, PromptWorkspacePanel};

const MAX_ATTACHMENTS: usize = 4;
const MAX_ATTACHMENT_BYTES: u64 = 10 * 1024 * 1024;

#[cfg(test)]
use attachments::is_supported_image;
