//! Provider-independent domain values and ports for Magenta.
//!
//! Conversation, generation, agent, workspace, persistence, and settings
//! contracts are shared by the application workflows and concrete adapters.
//! This crate has no GPUI, HTTP, or SQLite dependency. [`select_context`] keeps
//! recent whole turns within a model budget; [`MessageFailure`] carries safe,
//! durable failure details independently of technical error sources.

mod agent;
mod auth;
mod command;
mod context;
mod conversation;
mod error;
mod generation;
mod identifiers;
mod message;
mod models;
mod project;
mod settings;
mod storage;
mod workspace;

pub use agent::{
    AgentActivity, AgentActivityKind, AgentActivityRecord, AgentApprovalDecision,
    AgentApprovalRequest, AgentApprovalSubject, AgentContinuation, AgentProvider,
    AgentProviderEvent, AgentProviderStream, AgentRequest, AgentResumeRequest, AgentRunEvent,
    AgentRunStream, AgentToolCall, AgentToolDefinition, AgentToolOutput, AgentWorkspaceChange,
    ConversationMode, WorkspaceChangeKind, WorkspaceChangeState,
};
pub use auth::{
    AuthenticationFuture, AuthorizationSession, ProviderAccount, ProviderAuthenticator,
};
pub use command::{
    WorkspaceCommand, WorkspaceCommandError, WorkspaceCommandEvent, WorkspaceCommandOutputStream,
    WorkspaceCommandResult, WorkspaceCommandRunner, WorkspaceCommandStatus, WorkspaceCommandStream,
};
pub use context::{
    ContextBudgetReport, ContextTooLarge, estimate_agent_overhead, estimate_text_tokens,
    select_context,
};
pub use conversation::Conversation;
pub use error::{ProviderError, ProviderErrorDiagnostic, ProviderErrorKind};
pub use generation::{
    ChatProvider, EffortLevel, FinishReason, GenerationConfig, GenerationEvent, GenerationLimits,
    GenerationOutcome, GenerationRequest, GenerationStream, TokenUsage,
};
pub use identifiers::{AgentRunId, ConversationId, MessageId, ModelId, ProviderId};
pub use message::{
    Attachment, AttachmentDraft, Message, MessageFailure, MessageFailureCategory,
    MessageFailureDetail, MessageRole, MessageStatus,
};
pub use models::{ModelCatalog, ModelCatalogFuture, ModelDescriptor};
pub use project::{
    Project, ProjectStore, WorkspaceBrowser, WorkspaceDocument, WorkspaceEntry, WorkspaceEntryKind,
};
pub use settings::{
    AppSettings, AppearanceMode, FontChoice, MathFontStyle, SETTINGS_VERSION, SettingsError,
    SettingsFuture, SettingsStore, TypographySettings,
};
pub use storage::{
    BeginTurn, ConversationPage, ConversationSearchResult, ConversationStore, ConversationSummary,
    MessagePage, MessageSequence, PreparedTurn, StorageError, StorageErrorKind, StorageFuture,
    StoredMessage, Timestamp,
};
pub use workspace::{
    WorkspaceAccess, WorkspaceError, WorkspaceFuture, WorkspaceMutation, WorkspaceOperation,
    WorkspacePreview,
};
