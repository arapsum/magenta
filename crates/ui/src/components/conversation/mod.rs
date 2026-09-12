mod agent;
mod generation;
mod rendering;
mod state;
#[cfg(test)]
mod tests;

use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use futures_util::StreamExt as _;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _, StyledExt as _,
    button::{Button, ButtonVariants as _},
    clipboard::Clipboard,
    h_flex,
    scroll::ScrollableElement as _,
    text::{TextView, TextViewState, TextViewStyle},
    v_flex,
};
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, EventEmitter, FollowMode,
    InteractiveElement as _, IntoElement, ListAlignment, ListSizingBehavior, ListState,
    MouseButton, ObjectFit, ParentElement as _, Render, Role, StatefulInteractiveElement as _,
    Styled as _, StyledImage as _, Task, Window, div, img, linear_color_stop, linear_gradient,
    list, prelude::FluentBuilder as _, px, rems,
};
use magenta_application::AgentApprovalController;
use magenta_core::{
    AgentActivity, AgentActivityKind, AgentApprovalRequest, AgentApprovalSubject,
    AgentWorkspaceChange, Conversation, GenerationConfig, GenerationEvent, GenerationOutcome,
    GenerationStream, Message, MessageId, MessageRole, MessageStatus, ProviderError, ProviderId,
    WorkspaceCommand, WorkspaceCommandResult, WorkspaceCommandStatus,
};

use crate::components::{
    code_fence::{self, ContentSegment},
    inline_code::{self, MarkdownInlineCodePlugin},
    markdown,
    math::{self, FormulaKey, MarkdownMathPlugin, MathCache},
    premium_markdown::{
        PremiumCodeBlockPlugin, PremiumOrderedListPlugin, PremiumStepHeadingPlugin,
        conversation_text_style,
    },
    prompt_input::PromptComposer,
    provider_icon,
};

#[derive(Clone, Debug)]
pub struct ConversationThread {
    pub conversation: Conversation,
    pub messages: Vec<Message>,
}

const MESSAGE_MAX_WIDTH: gpui_kit::Pixels = px(760.);
const COMPOSER_MAX_WIDTH: gpui_kit::Pixels = px(800.);
const USER_MESSAGE_MAX_WIDTH: gpui_kit::Pixels = px(520.);
const LIST_OVERDRAW: gpui_kit::Pixels = px(640.);
const GENERATION_CLOCK_INTERVAL: Duration = Duration::from_secs(1);
const MAX_RENDERED_MESSAGES: usize = 150;

#[derive(Clone, Debug, Default, Eq, PartialEq, gpui_kit::Action)]
#[action(namespace = magenta)]
struct CloseAttachmentPreview;

#[derive(Debug, thiserror::Error)]
#[error("provider stream ended before completion")]
struct IncompleteGeneration;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum GenerationPhase {
    Connecting,
    Thinking,
    Responding,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum PageLoadState {
    #[default]
    Idle,
    Earlier,
    Newer,
}

impl GenerationPhase {
    const fn label(self) -> &'static str {
        match self {
            Self::Connecting => "Connecting",
            Self::Thinking => "Thinking",
            Self::Responding => "Responding",
        }
    }
}

struct GenerationProgress {
    message_id: MessageId,
    provider: ProviderId,
    configuration: Option<GenerationConfig>,
    phase: GenerationPhase,
    started_at: Instant,
    provider_started_at: Option<Instant>,
    first_text_at: Option<Instant>,
}

impl GenerationProgress {
    fn new(
        message_id: MessageId,
        provider: ProviderId,
        configuration: Option<GenerationConfig>,
    ) -> Self {
        Self {
            message_id,
            provider,
            configuration,
            phase: GenerationPhase::Connecting,
            started_at: Instant::now(),
            provider_started_at: None,
            first_text_at: None,
        }
    }

    fn elapsed(&self) -> Duration {
        self.started_at.elapsed()
    }

    fn provider_started_ms(&self) -> Option<u64> {
        self.provider_started_at
            .map(|instant| duration_millis(instant.duration_since(self.started_at)))
    }

    fn first_text_ms(&self) -> Option<u64> {
        self.first_text_at
            .map(|instant| duration_millis(instant.duration_since(self.started_at)))
    }
}

#[derive(Clone, Debug)]
pub enum ConversationViewEvent {
    GenerationStarted,
    GenerationFinished(Message),
    LoadEarlier,
    LoadNewer,
    ReturnToLatest,
    Regenerate(MessageId),
    Retry(MessageId),
    PrepareContinue(MessageId),
    ChooseModelForRetry(MessageId),
    FocusComposer,
    OpenProviderSettings,
    WorkspaceChange(AgentWorkspaceChange),
    WorkspaceInvalidated,
}

#[derive(Clone, Debug)]
struct LiveCommand {
    command: WorkspaceCommand,
    stdout: String,
    stderr: String,
    result: Option<WorkspaceCommandResult>,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum ActivitySection {
    Commands,
    ToolCalls,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ActivitySectionOverride {
    Open,
    Closed,
}

struct RenderedMessage {
    message: Message,
    created_at: magenta_core::Timestamp,
    markdown: Option<Entity<TextViewState>>,
    markdown_source: Option<String>,
    user_segments: Vec<RenderedUserSegment>,
    sequence: Option<magenta_core::MessageSequence>,
    omitted_context_messages: usize,
}

#[derive(Clone)]
struct AttachmentPreview {
    path: PathBuf,
    name: String,
}

enum RenderedUserSegment {
    Text(String),
    Code {
        source_start: usize,
        markdown: Entity<TextViewState>,
    },
}

pub struct ConversationView {
    composer: Entity<PromptComposer>,
    conversation: Option<Conversation>,
    messages: Vec<RenderedMessage>,
    list_state: ListState,
    generation: u64,
    streaming_message: Option<MessageId>,
    generation_task: Option<Task<()>>,
    generation_clock_task: Option<Task<()>>,
    generation_progress: Option<GenerationProgress>,
    agent_controller: Option<AgentApprovalController>,
    pending_agent_approval: Option<(MessageId, AgentApprovalRequest)>,
    live_commands: HashMap<(MessageId, String), LiveCommand>,
    activity_section_overrides: HashMap<(MessageId, ActivitySection), ActivitySectionOverride>,
    expanded_agent_activity_calls: HashSet<(MessageId, String)>,
    older_cursor: Option<magenta_core::MessageSequence>,
    has_older: bool,
    page_load: PageLoadState,
    newer_cursor: Option<magenta_core::MessageSequence>,
    has_newer: bool,
    origins: HashMap<MessageId, GenerationConfig>,
    math_cache: Arc<MathCache>,
    math_tasks: HashMap<FormulaKey, Task<()>>,
    attachment_preview: Option<AttachmentPreview>,
}

type ConversationContext<'a> = Context<'a, ConversationView>;

impl EventEmitter<ConversationViewEvent> for ConversationView {}

fn duration_millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

fn format_elapsed(duration: Duration) -> String {
    let seconds = duration.as_secs();
    if seconds < 60 {
        return format!("{seconds}s");
    }

    let minutes = seconds / 60;
    if minutes < 60 {
        return format!("{minutes}m {:02}s", seconds % 60);
    }

    format!("{}h {:02}m", minutes / 60, minutes % 60)
}

fn trace_generation_terminal(progress: &GenerationProgress, status: &'static str) {
    let model = progress
        .configuration
        .as_ref()
        .map_or("unknown", |configuration| configuration.model.0.as_str());
    let effort = progress
        .configuration
        .as_ref()
        .map_or("unknown", |configuration| configuration.effort.label());
    tracing::info!(
        provider = %progress.provider.0,
        model,
        effort,
        phase = progress.phase.label(),
        status,
        elapsed_ms = duration_millis(progress.elapsed()),
        provider_started_ms = progress.provider_started_ms(),
        first_text_ms = progress.first_text_ms(),
        operation = "conversation.generate",
        "generation finished"
    );
}

impl ConversationView {
    fn render_thread_footer(&self, cx: &Context<'_, Self>) -> AnyElement {
        if self.has_newer {
            h_flex()
                .flex_none()
                .w_full()
                .justify_center()
                .gap(px(8.))
                .px(px(24.))
                .pt(px(10.))
                .pb(px(18.))
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(cx.theme().muted_foreground)
                        .child("Viewing older messages"),
                )
                .child(
                    Button::new("load-newer-messages")
                        .ghost()
                        .small()
                        .label(if self.page_load == PageLoadState::Newer {
                            "Loading newer messages…"
                        } else {
                            "Load newer"
                        })
                        .disabled(self.page_load == PageLoadState::Newer)
                        .on_click(cx.listener(|_, _, _, cx| {
                            cx.emit(ConversationViewEvent::LoadNewer);
                        })),
                )
                .child(
                    Button::new("return-to-latest")
                        .outline()
                        .small()
                        .label("Return to latest")
                        .on_click(cx.listener(|_, _, _, cx| {
                            cx.emit(ConversationViewEvent::ReturnToLatest);
                        })),
                )
                .into_any_element()
        } else {
            div()
                .flex_none()
                .w_full()
                .px(px(24.))
                .pt(px(12.))
                .pb(px(22.))
                .child(
                    div()
                        .w_full()
                        .max_w(COMPOSER_MAX_WIDTH)
                        .mx_auto()
                        .child(self.composer.clone()),
                )
                .into_any_element()
        }
    }
}

impl Render for ConversationView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let view = cx.entity();
        let list_state = self.list_state.clone();
        v_flex()
            .relative()
            .size_full()
            .min_w_0()
            .bg(cx.theme().tokens.background.background)
            .text_color(cx.theme().foreground)
            .child(
                img("icons/surface-glow.svg")
                    .absolute()
                    .inset_0()
                    .size_full()
                    .object_fit(ObjectFit::Cover),
            )
            .when(self.has_older, |this| {
                this.child(
                    Button::new("load-earlier-messages")
                        .ghost()
                        .small()
                        .label(if self.page_load == PageLoadState::Earlier {
                            "Loading earlier messages…"
                        } else {
                            "Load earlier messages"
                        })
                        .disabled(self.page_load == PageLoadState::Earlier)
                        .accessibility_id("load-earlier-messages")
                        .on_click(
                            cx.listener(|_, _, _, cx| cx.emit(ConversationViewEvent::LoadEarlier)),
                        ),
                )
            })
            .child(
                list(list_state, move |index, window, cx| {
                    view.read(cx).render_message(index, window, cx, &view)
                })
                .relative()
                .with_sizing_behavior(ListSizingBehavior::Auto)
                .flex_grow_1()
                .min_h_0()
                .w_full(),
            )
            .child(self.render_thread_footer(cx))
            .when_some(self.attachment_preview_overlay(cx), |this, overlay| {
                this.child(overlay)
            })
    }
}
