use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
    time::Duration,
};

use gpui::{
    App, AppContext as _, Context, Entity, EventEmitter, Focusable as _, PathPromptOptions,
    SharedString, Subscription, Task, Window,
};
use gpui_component::{
    WindowExt,
    input::{InputEvent, TextareaState},
    notification::{Notification, NotificationType},
    text::TextViewState,
};
use magenta_core::{ConversationMode, EffortLevel, GenerationConfig, ModelDescriptor};

use super::{MAX_ATTACHMENT_BYTES, MAX_ATTACHMENTS};
use crate::{MagentaError, components::code_fence, notification_for_error};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ReferenceImage {
    pub(super) id: u64,
    pub(super) path: PathBuf,
    pub(super) name: SharedString,
}

impl ReferenceImage {
    fn new(path: PathBuf) -> Self {
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
    WorkspaceSelected(PathBuf),
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum AgentCapability {
    #[default]
    Unavailable,
    Files,
    Commands,
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
    pub(super) model: Option<ModelDescriptor>,
    pub(super) effort: Option<EffortLevel>,
    pub(super) mode: ConversationMode,
    pub(super) workspace_root: Option<PathBuf>,
    pub(super) agent_capability: AgentCapability,
    pub(super) generating: bool,
    storage_ready: bool,
    pub(super) attachments: Vec<ReferenceImage>,
    attachment_task: Option<Task<()>>,
    workspace_task: Option<Task<()>>,
    preview_task: Option<Task<()>>,
    preview_generation: u64,
    pub(super) subscriptions: Vec<Subscription>,
}

impl PromptComposer {
    pub fn new(window: &mut Window, cx: &mut Context<'_, Self>) -> Self {
        let input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("Ask Magenta anything…")
                .auto_grow(2, 5)
                .submit_on_enter(true)
        });
        let preview = cx.new(|cx| TextViewState::markdown("", cx));

        let subscriptions = vec![cx.subscribe_in(
            &input,
            window,
            |composer, _, event: &InputEvent, window, cx| match event {
                InputEvent::Change => composer.schedule_code_preview(window, cx),
                InputEvent::Focus | InputEvent::Blur => cx.notify(),
                InputEvent::PressEnter { shift: false, .. } => composer.submit(cx),
                InputEvent::PressEnter { shift: true, .. } => {
                    composer.handle_shift_enter(window, cx);
                }
            },
        )];

        Self {
            input,
            preview,
            preview_source: String::new(),
            preview_line_count: 0,
            models: Vec::new(),
            model: None,
            effort: None,
            mode: ConversationMode::Chat,
            workspace_root: None,
            agent_capability: AgentCapability::Unavailable,
            generating: false,
            storage_ready: true,
            attachments: Vec::new(),
            attachment_task: None,
            workspace_task: None,
            preview_task: None,
            preview_generation: 0,
            subscriptions,
        }
    }

    pub fn focus(&self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.input.focus_handle(cx).focus(window, cx);
    }

    pub fn set_configuration(
        &mut self,
        configuration: &GenerationConfig,
        cx: &mut Context<'_, Self>,
    ) {
        let model = self
            .models
            .iter()
            .find(|model| {
                model.provider.eq(&configuration.provider) && model.id.eq(&configuration.model)
            })
            .cloned()
            .or_else(|| self.models.first().cloned())
            .unwrap_or_else(|| ModelDescriptor {
                provider: configuration.provider.clone(),
                id: configuration.model.clone(),
                display_name: configuration.model.0.clone(),
                description: None,
                priority: 0,
                default_effort: configuration.effort.clone(),
                supported_efforts: EffortLevel::ALL.to_vec(),
                limits: configuration.limits,
            });
        let uses_requested_model =
            model.provider.eq(&configuration.provider) && model.id.eq(&configuration.model);
        self.effort = model
            .supported_efforts
            .contains(&configuration.effort)
            .then_some(configuration.effort.clone())
            .filter(|_| uses_requested_model)
            .or_else(|| Some(model.default_effort.clone()));
        self.model = Some(model);
        cx.notify();
    }

    pub(crate) fn set_models(&mut self, models: Vec<ModelDescriptor>, cx: &mut Context<'_, Self>) {
        let selected = self.model.as_ref().and_then(|selected| {
            models
                .iter()
                .find(|model| model.provider == selected.provider && model.id == selected.id)
                .cloned()
        });
        self.models = models;
        self.model = selected.or_else(|| self.models.first().cloned());
        let current_effort = self.effort.clone();
        self.effort = self.model.as_ref().map(|model| {
            current_effort
                .filter(|effort| model.supported_efforts.contains(effort))
                .unwrap_or_else(|| model.default_effort.clone())
        });
        cx.notify();
    }

    pub fn set_generating(&mut self, generating: bool, cx: &mut Context<'_, Self>) {
        if self.generating != generating {
            self.generating = generating;
            cx.notify();
        }
    }

    pub(crate) fn set_agent_capability(
        &mut self,
        capability: AgentCapability,
        cx: &mut Context<'_, Self>,
    ) {
        self.agent_capability = capability;
        if !capability.available() && self.mode == ConversationMode::Agent {
            self.mode = ConversationMode::Chat;
        }
        cx.notify();
    }

    pub(crate) fn set_conversation_context(
        &mut self,
        mode: ConversationMode,
        workspace_root: Option<PathBuf>,
        cx: &mut Context<'_, Self>,
    ) {
        self.mode = if mode == ConversationMode::Agent && !self.agent_capability.available() {
            ConversationMode::Chat
        } else {
            mode
        };
        self.workspace_root = workspace_root;
        cx.notify();
    }

    pub(crate) fn activate_workspace(&mut self, root: PathBuf, cx: &mut Context<'_, Self>) {
        if self.agent_capability.available() {
            self.mode = ConversationMode::Agent;
            self.workspace_root = Some(root);
            cx.notify();
        }
    }

    pub(crate) fn set_storage_ready(&mut self, ready: bool, cx: &mut Context<'_, Self>) {
        self.storage_ready = ready;
        cx.notify();
    }

    pub fn clear_after_submit(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.preview_generation = self.preview_generation.wrapping_add(1);
        self.preview_task.take();
        self.clear_code_preview(cx);
        self.attachments.clear();
        cx.notify();
    }

    pub(crate) fn clear_submitted(
        &mut self,
        submitted: &PromptRequest,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let same_prompt = self.input.read(cx).value().trim() == submitted.prompt.as_ref();
        let same_attachments = self
            .attachments
            .iter()
            .map(|image| &image.path)
            .eq(submitted.attachments.iter());
        if same_prompt && same_attachments {
            self.clear_after_submit(window, cx);
        }
    }

    fn schedule_code_preview(&mut self, window: &Window, cx: &mut Context<'_, Self>) {
        self.preview_generation = self.preview_generation.wrapping_add(1);
        let generation = self.preview_generation;
        self.preview_task.take();

        let source = self.input.read(cx).value().to_string();
        if !source.contains("```") {
            self.clear_code_preview(cx);
            cx.notify();
            return;
        }

        self.preview_task = Some(cx.spawn_in(window, async move |composer, window| {
            window
                .background_executor()
                .timer(Duration::from_millis(60))
                .await;
            let blocks = window
                .background_executor()
                .spawn(async move { code_fence::fenced_blocks(&source) })
                .await;

            _ = composer.update_in(window, |composer, _, cx| {
                if composer.preview_generation != generation {
                    return;
                }
                composer.preview_task = None;
                composer.apply_code_preview(&blocks, cx);
            });
        }));
        cx.notify();
    }

    fn apply_code_preview(
        &mut self,
        blocks: &[code_fence::FencedCodeBlock],
        cx: &mut Context<'_, Self>,
    ) {
        let source = code_fence::preview_markdown(blocks);
        if self.preview_source == source {
            return;
        }

        self.preview_line_count = source.lines().count();
        self.preview_source.clone_from(&source);
        self.preview
            .update(cx, |preview, cx| preview.set_text(&source, cx));
        cx.notify();
    }

    fn clear_code_preview(&mut self, cx: &mut Context<'_, Self>) {
        if self.preview_source.is_empty() {
            return;
        }
        self.preview_source.clear();
        self.preview_line_count = 0;
        self.preview
            .update(cx, |preview, cx| preview.set_text("", cx));
    }

    pub(super) fn handle_shift_enter(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let (value, cursor) = {
            let input = self.input.read(cx);
            (input.value(), input.cursor())
        };
        let Some(auto_close) = code_fence::opening_fence_after_newline(&value, cursor) else {
            self.schedule_code_preview(window, cx);
            return;
        };

        self.input.update(cx, |input, cx| {
            input.insert(auto_close.insertion, window, cx);
            input.set_selected_range(cursor..cursor, cx);
        });
        self.schedule_code_preview(window, cx);
    }

    fn has_content(&self, cx: &App) -> bool {
        !self.input.read(cx).value().trim().is_empty() || !self.attachments.is_empty()
    }

    pub(super) fn is_ready(&self, cx: &App) -> bool {
        let workspace_ready = self.mode == ConversationMode::Chat
            || (self.agent_capability.available()
                && self
                    .workspace_root
                    .as_deref()
                    .is_some_and(std::path::Path::is_dir));
        self.storage_ready
            && self.has_content(cx)
            && self.model.is_some()
            && self.effort.is_some()
            && workspace_ready
    }

    pub(super) fn select_mode(&mut self, mode: ConversationMode, cx: &mut Context<'_, Self>) {
        if mode == ConversationMode::Agent && !self.agent_capability.available() {
            return;
        }
        if self.mode != mode {
            self.mode = mode;
            cx.notify();
        }
    }

    pub(super) fn choose_workspace(&mut self, window: &Window, cx: &Context<'_, Self>) {
        if self.mode != ConversationMode::Agent || !self.agent_capability.available() {
            return;
        }

        let picker = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Choose workspace".into()),
        });
        self.workspace_task = Some(cx.spawn_in(window, async move |composer, window| {
            let selection = match picker.await {
                Ok(Ok(paths)) => paths,
                Ok(Err(source)) => {
                    _ = composer.update_in(window, |composer, window, cx| {
                        composer.workspace_task = None;
                        let error = MagentaError::AttachmentPicker { source };
                        window.push_notification(notification_for_error(&error), cx);
                    });
                    return;
                }
                Err(_) => return,
            };
            _ = composer.update_in(window, |composer, _, cx| {
                composer.workspace_task = None;
                composer.workspace_root = selection
                    .and_then(|paths| paths.into_iter().next())
                    .filter(|path| path.is_dir());
                if let Some(root) = composer.workspace_root.clone() {
                    cx.emit(PromptComposerEvent::WorkspaceSelected(root));
                }
                cx.notify();
            });
        }));
    }

    pub(super) fn select_model(&mut self, model: ModelDescriptor, cx: &mut Context<'_, Self>) {
        if self.model.as_ref() != Some(&model) {
            self.effort = Some(model.default_effort.clone());
            self.model = Some(model);
            cx.notify();
        }
    }

    pub(super) fn select_effort(&mut self, effort: EffortLevel, cx: &mut Context<'_, Self>) {
        if self
            .model
            .as_ref()
            .is_some_and(|model| model.supported_efforts.contains(&effort))
            && self.effort.as_ref() != Some(&effort)
        {
            self.effort = Some(effort);
            cx.notify();
        }
    }

    pub(super) fn choose_attachments(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.attachments.len() >= MAX_ATTACHMENTS {
            window.push_notification(
                Notification::new()
                    .title("Four images already attached")
                    .message("Remove an image before adding another attachment.")
                    .with_type(NotificationType::Warning),
                cx,
            );
            return;
        }

        let picker = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Add images".into()),
        });
        self.attachment_task = Some(cx.spawn_in(window, async move |composer, window| {
            let selection = match picker.await {
                Ok(Ok(paths)) => paths,
                Ok(Err(source)) => {
                    _ = composer.update_in(window, |composer, window, cx| {
                        composer.attachment_task = None;
                        let error = MagentaError::AttachmentPicker { source };
                        window.push_notification(notification_for_error(&error), cx);
                    });
                    return;
                }
                Err(_) => return,
            };
            let Some(paths) = selection else {
                _ = composer.update_in(window, |composer, _, _| composer.attachment_task = None);
                return;
            };

            let paths = window
                .background_executor()
                .spawn(async move {
                    paths
                        .into_iter()
                        .map(|path| {
                            let metadata = std::fs::metadata(&path).ok();
                            let readable =
                                metadata.as_ref().is_some_and(std::fs::Metadata::is_file)
                                    && std::fs::File::open(&path).is_ok();
                            let byte_size = metadata.map(|metadata| metadata.len());
                            (path, readable, byte_size)
                        })
                        .collect::<Vec<_>>()
                })
                .await;

            _ = composer.update_in(window, |composer, window, cx| {
                composer.add_attachments(paths, window, cx);
                composer.attachment_task = None;
            });
        }));
    }

    fn add_attachments(
        &mut self,
        paths: Vec<(PathBuf, bool, Option<u64>)>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let mut unsupported = 0;
        let mut unreadable = 0;
        let mut too_large = 0;
        let mut duplicates = 0;
        let mut overflow = 0;

        for (path, readable, byte_size) in paths {
            if !is_supported_image(&path) {
                unsupported += 1;
            } else if !readable {
                unreadable += 1;
            } else if byte_size.is_none_or(|size| size > MAX_ATTACHMENT_BYTES) {
                too_large += 1;
            } else if self
                .attachments
                .iter()
                .any(|attachment| attachment.path == path)
            {
                duplicates += 1;
            } else if self.attachments.len() >= MAX_ATTACHMENTS {
                overflow += 1;
            } else {
                self.attachments.push(ReferenceImage::new(path));
            }
        }

        let skipped = unsupported + unreadable + too_large + duplicates + overflow;
        if skipped > 0 {
            let message = format!(
                concat!(
                    "Skipped {skipped} file(s): {unsupported} unsupported, ",
                    "{unreadable} unreadable, {too_large} over 10 MiB, {duplicates} duplicate, ",
                    "{overflow} over the four-image limit."
                ),
                skipped = skipped,
                unsupported = unsupported,
                unreadable = unreadable,
                too_large = too_large,
                duplicates = duplicates,
                overflow = overflow
            );
            window.push_notification(
                Notification::new()
                    .title("Some images were not added")
                    .message(message)
                    .with_type(NotificationType::Warning),
                cx,
            );
        }
        cx.notify();
    }

    pub(super) fn remove_attachment(&mut self, path: &Path, cx: &mut Context<'_, Self>) {
        let before = self.attachments.len();
        self.attachments
            .retain(|attachment| attachment.path != *path);
        if self.attachments.len() != before {
            cx.notify();
        }
    }

    pub(super) fn request(&self, cx: &App) -> Option<PromptRequest> {
        if !self.is_ready(cx) {
            return None;
        }
        let model = self.model.as_ref()?;
        let effort = self.effort.clone()?;
        Some(PromptRequest {
            prompt: self.input.read(cx).value().trim().to_owned().into(),
            generation: GenerationConfig::new(model.provider.clone(), model.id.clone(), effort)
                .with_limits(model.limits),
            attachments: self
                .attachments
                .iter()
                .map(|attachment| attachment.path.clone())
                .collect(),
            mode: self.mode.clone(),
            workspace_root: self.workspace_root.clone(),
        })
    }

    pub(super) fn submit(&self, cx: &mut Context<'_, Self>) {
        if !self.generating
            && let Some(request) = self.request(cx)
        {
            cx.emit(PromptComposerEvent::Submit(request));
        }
    }

    pub(super) fn cancel(&self, cx: &mut Context<'_, Self>) {
        if self.generating {
            cx.emit(PromptComposerEvent::Cancel);
        }
    }
}

impl EventEmitter<PromptComposerEvent> for PromptComposer {}

pub(super) fn is_supported_image(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "png" | "jpg" | "jpeg" | "webp"
            )
        })
}
