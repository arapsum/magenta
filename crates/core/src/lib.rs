//! Provider-independent conversation values shared by Magenta's UI and
//! future storage/provider adapters.

mod auth;
mod conversation;
mod error;
mod generation;
mod identifiers;
mod message;
mod models;
mod settings;
mod storage;

pub use auth::{
    AuthenticationFuture, AuthorizationSession, ProviderAccount, ProviderAuthenticator,
};
pub use conversation::Conversation;
pub use error::{ProviderError, ProviderErrorKind};
pub use generation::{
    ChatProvider, EffortLevel, FinishReason, GenerationConfig, GenerationEvent, GenerationOutcome,
    GenerationRequest, GenerationStream, TokenUsage,
};
pub use identifiers::{ConversationId, MessageId, ModelId, ProviderId};
pub use message::{Attachment, Message, MessageRole, MessageStatus};
pub use models::{ModelCatalog, ModelCatalogFuture, ModelDescriptor};
pub use settings::{
    AppSettings, AppearanceMode, FontChoice, MathFontStyle, SETTINGS_VERSION, SettingsError,
    SettingsFuture, SettingsStore, TypographySettings,
};
pub use storage::{
    BeginTurn, ConversationPage, ConversationStore, ConversationSummary, MessagePage,
    MessageSequence, PreparedTurn, StorageError, StorageErrorKind, StorageFuture, StoredMessage,
    Timestamp,
};
