use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    path::PathBuf,
    time::Duration,
};

use gpui_kit::component::{
    input::{InputEvent, TextareaState},
    text::TextViewState,
};
use gpui_kit::{
    App, AppContext as _, Context, Entity, EventEmitter, Focusable as _, PathPromptOptions,
    SharedString, Subscription, Task, Window,
};
use magenta_core::{
    ConversationMode, EffortLevel, GenerationConfig, GenerationSettings, MessageId, ModelDescriptor,
};

use crate::{ErrorPresentation, MagentaError, components::code_fence};

mod generation;
mod lifecycle;
mod modes;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ReferenceImage {
    pub(super) id: u64,
    pub(super) path: PathBuf,
    pub(super) name: SharedString,
}

impl ReferenceImage {
    pub(super) fn new(path: PathBuf) -> Self {
        let mut hasher = DefaultHasher::new();
        path.hash(&mut hasher);
        let id = hasher.finish();
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("Reference image")
            .to_owned()
            .into();

        Self { id, path, name }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PromptRequest {
    pub prompt: SharedString,
    pub generation: GenerationConfig,
    pub attachments: Vec<PathBuf>,
    pub mode: ConversationMode,
    pub workspace_root: Option<PathBuf>,
}

#[derive(Clone, Debug)]
pub enum PromptComposerEvent {
    Submit(PromptRequest),
    Cancel,
    ModeChanged,
    WorkspaceSelected(PathBuf),
    OpenWorkspacePanel(PromptWorkspacePanel),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PromptWorkspacePanel {
    Files,
    Changes,
}

#[derive(Clone, Debug, Default)]
struct ModeDraft {
    prompt: String,
    attachments: Vec<ReferenceImage>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum AgentCapability {
    #[default]
    Unavailable,
    Files,
    Commands,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum GenerationSelectionOrigin {
    Automatic,
    ConfiguredDefault,
    Manual,
    PersistedConversation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ModelCatalogState {
    NotLoaded,
    Loaded,
}

impl ModelCatalogState {
    pub(super) const fn is_loaded(self) -> bool {
        matches!(self, Self::Loaded)
    }
}

#[derive(Clone, Debug)]
pub(super) struct ModeGenerationState {
    pub(super) model: Option<ModelDescriptor>,
    pub(super) effort: Option<EffortLevel>,
    pub(super) origin: GenerationSelectionOrigin,
    pub(super) model_available: bool,
    pub(super) effort_available: bool,
    pub(super) persisted: Option<GenerationConfig>,
}

impl Default for ModeGenerationState {
    fn default() -> Self {
        Self {
            model: None,
            effort: None,
            origin: GenerationSelectionOrigin::Automatic,
            model_available: false,
            effort_available: false,
            persisted: None,
        }
    }
}

impl AgentCapability {
    pub(super) const fn available(self) -> bool {
        !matches!(self, Self::Unavailable)
    }

    pub(super) const fn commands(self) -> bool {
        matches!(self, Self::Commands)
    }
}

pub struct PromptComposer {
    pub(super) input: Entity<TextareaState>,
    pub(super) preview: Entity<TextViewState>,
    pub(super) preview_source: String,
    pub(super) preview_line_count: usize,
    pub(super) models: Vec<ModelDescriptor>,
    pub(super) model_catalog_state: ModelCatalogState,
    pub(super) model: Option<ModelDescriptor>,
    pub(super) effort: Option<EffortLevel>,
    pub(super) chat_generation: ModeGenerationState,
    pub(super) work_generation: ModeGenerationState,
    pub(super) generation_settings: GenerationSettings,
    pub(super) mode: ConversationMode,
    input_mode: ConversationMode,
    pub(super) workspace_root: Option<PathBuf>,
    pub(super) agent_capability: AgentCapability,
    pub(super) generating: bool,
    storage_ready: bool,
    submission_ready: bool,
    pub(super) attachments: Vec<ReferenceImage>,
    chat_draft: ModeDraft,
    work_draft: ModeDraft,
    retry_target: Option<MessageId>,
    pub(super) inline_error: Option<ErrorPresentation>,
    blocking_error: Option<ErrorPresentation>,
    pub(super) attachment_task: Option<Task<()>>,
    workspace_task: Option<Task<()>>,
    preview_task: Option<Task<()>>,
    preview_generation: u64,
    pub(super) subscriptions: Vec<Subscription>,
}

impl EventEmitter<PromptComposerEvent> for PromptComposer {}
