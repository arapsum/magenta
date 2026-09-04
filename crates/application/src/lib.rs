//! Application workflows that coordinate Magenta's domain values and ports.

mod error;
mod send_message;

pub use error::SendMessageError;
pub use send_message::{MessageIds, PendingGeneration, SendMessage, SendMessageInput, SendTarget};

pub type Result<T> = std::result::Result<T, SendMessageError>;
