//! Provider-independent conversation values shared by Magenta's UI and
//! future storage/provider adapters.

mod conversation;
mod error;
mod generation;
mod identifiers;
mod message;

pub use conversation::Conversation;
pub use error::ProviderError;
pub use generation::{
    ChatProvider, EffortLevel, GenerationConfig, GenerationEvent, GenerationRequest,
    GenerationStream,
};
pub use identifiers::{ConversationId, MessageId, ModelId, ProviderId};
pub use message::{Attachment, Message, MessageRole, MessageStatus};
