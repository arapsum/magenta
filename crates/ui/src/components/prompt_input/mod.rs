mod render;
mod state;
#[cfg(test)]
mod tests;

pub use state::AgentCapability;
pub use state::{PromptComposer, PromptComposerEvent, PromptRequest};

const MAX_ATTACHMENTS: usize = 4;
const MAX_ATTACHMENT_BYTES: u64 = 10 * 1024 * 1024;

#[cfg(test)]
use state::is_supported_image;
