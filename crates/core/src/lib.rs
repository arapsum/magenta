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
    ChatProvider, EffortLevel, FinishReason, GenerationConfig, GenerationEvent, GenerationOutcome,
    GenerationRequest, GenerationStream, TokenUsage,
};
pub use identifiers::{ConversationId, MessageId, ModelId, ProviderId};
pub use message::{Attachment, Message, MessageRole, MessageStatus};
