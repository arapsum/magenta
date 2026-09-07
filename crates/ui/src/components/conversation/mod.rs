mod agent;
mod generation;
mod rendering;
mod state;
#[cfg(test)]
mod tests;

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use futures_util::StreamExt as _;
use gpui::{
    AnyElement, App, AppContext as _, Context, Entity, EventEmitter, FollowMode,
    InteractiveElement as _, IntoElement, ListAlignment, ListSizingBehavior, ListState,
    MouseButton, ObjectFit, ParentElement as _, Render, Role, StatefulInteractiveElement as _,
    Styled as _, StyledImage as _, Task, Window, div, img, list, prelude::FluentBuilder as _, px,
    rems,
};
use gpui_component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _, StyledExt as _,
    WindowExt as _,
    button::{Button, ButtonVariants as _},
    clipboard::Clipboard,
    h_flex,
    scroll::ScrollableElement as _,
    text::{TextView, TextViewState, TextViewStyle},
    v_flex,
};
use magenta_application::AgentApprovalController;
use magenta_core::{
    AgentActivityKind, AgentApprovalRequest, Conversation, GenerationConfig, GenerationEvent,
    GenerationOutcome, GenerationStream, Message, MessageId, MessageRole, MessageStatus,
    ProviderError, ProviderId,
};

use crate::components::{
    code_fence::{self, ContentSegment},
    inline_code::{self, MarkdownInlineCodePlugin},
    markdown,
    math::{self, FormulaKey, MarkdownMathPlugin, MathCache},
    prompt_input::PromptComposer,
};
use crate::{MagentaError, notification_for_error};

#[derive(Clone, Debug)]
pub struct ConversationThread {
    pub conversation: Conversation,
    pub messages: Vec<Message>,
}

const MESSAGE_MAX_WIDTH: gpui::Pixels = px(760.);
const USER_MESSAGE_MAX_WIDTH: gpui::Pixels = px(560.);
const LIST_OVERDRAW: gpui::Pixels = px(640.);
const GENERATION_CLOCK_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Clone, Debug, Default, Eq, PartialEq, gpui::Action)]
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
    Regenerate(MessageId),
}

struct RenderedMessage {
    message: Message,
    markdown: Option<Entity<TextViewState>>,
    markdown_source: Option<String>,
    user_segments: Vec<RenderedUserSegment>,
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
    older_cursor: Option<magenta_core::MessageSequence>,
    has_older: bool,
    loading_earlier: bool,
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
            .when(self.has_older, |this| {
                this.child(
                    Button::new("load-earlier-messages")
                        .ghost()
                        .small()
                        .label(if self.loading_earlier {
                            "Loading earlier messages…"
                        } else {
                            "Load earlier messages"
                        })
                        .disabled(self.loading_earlier)
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
                .with_sizing_behavior(ListSizingBehavior::Auto)
                .flex_grow_1()
                .min_h_0()
                .w_full(),
            )
            .child(
                div()
                    .flex_none()
                    .w_full()
                    .px(px(24.))
                    .pt(px(10.))
                    .pb(px(18.))
                    .child(
                        div()
                            .w_full()
                            .max_w(MESSAGE_MAX_WIDTH)
                            .mx_auto()
                            .child(self.composer.clone()),
                    ),
            )
            .when_some(self.attachment_preview_overlay(cx), |this, overlay| {
                this.child(overlay)
            })
    }
}
