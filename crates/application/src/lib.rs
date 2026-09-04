//! Application workflows that coordinate Magenta's domain values and ports.

mod error;
mod regenerate_message;
mod send_message;

pub use error::{RegenerateMessageError, SendMessageError};
pub use regenerate_message::{PendingRegeneration, RegenerateMessage, RegenerateMessageInput};
pub use send_message::{MessageIds, PendingGeneration, SendMessage, SendMessageInput, SendTarget};
