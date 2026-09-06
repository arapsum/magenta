use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use futures_util::StreamExt as _;
use gpui::{
    AnyElement, App, AppContext as _, Context, Entity, EventEmitter, FollowMode, IntoElement,
    ListAlignment, ListSizingBehavior, ListState, ParentElement as _, Render, Styled as _, Task,
    Window, div, list, prelude::FluentBuilder as _, px, rems,
};
use gpui_component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _, StyledExt as _,
    WindowExt as _,
    button::{Button, ButtonVariants as _},
    clipboard::Clipboard,
    h_flex,
    text::{TextView, TextViewState, TextViewStyle},
    v_flex,
};
use magenta_core::{
    Conversation, GenerationConfig, GenerationEvent, GenerationOutcome, GenerationStream, Message,
    MessageId, MessageRole, MessageStatus, ProviderError, ProviderId,
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
    older_cursor: Option<magenta_core::MessageSequence>,
    has_older: bool,
    loading_earlier: bool,
    origins: HashMap<MessageId, GenerationConfig>,
    math_cache: Arc<MathCache>,
    math_tasks: HashMap<FormulaKey, Task<()>>,
}

type ConversationContext<'a> = Context<'a, ConversationView>;

impl ConversationView {
    pub(crate) fn load_page(
        &mut self,
        loaded: magenta_core::ConversationPage,
        cx: &mut Context<'_, Self>,
    ) {
        let origins = loaded
            .page
            .messages
            .iter()
            .map(|item| (item.message.id, item.generation.clone()))
            .collect();
        self.load(
            ConversationThread {
                conversation: loaded.conversation,
                messages: loaded
                    .page
                    .messages
                    .into_iter()
                    .map(|item| item.message)
                    .collect(),
            },
            cx,
        );
        self.origins = origins;
        self.has_older = loaded.page.has_older;
        self.older_cursor = loaded.page.older_cursor;
        cx.notify();
    }

    pub(crate) fn earlier_cursor(&self) -> Option<magenta_core::MessageSequence> {
        self.has_older.then_some(self.older_cursor).flatten()
    }

    pub(crate) fn set_loading_earlier(&mut self, loading: bool, cx: &mut Context<'_, Self>) {
        self.loading_earlier = loading;
        cx.notify();
    }

    pub(crate) fn prepend_page(
        &mut self,
        page: magenta_core::MessagePage,
        cx: &mut Context<'_, Self>,
    ) {
        let mut anchor = self.list_state.logical_scroll_top();
        let mut earlier = Vec::new();
        for item in page.messages {
            if self
                .messages
                .iter()
                .any(|loaded| loaded.message.id == item.message.id)
            {
                continue;
            }
            self.origins.insert(item.message.id, item.generation);
            earlier.push(Self::rendered_message(item.message, cx));
        }
        let count = earlier.len();
        earlier.append(&mut self.messages);
        self.messages = earlier;
        self.list_state.splice(0..0, count);
        anchor.item_ix += count;
        self.list_state.scroll_to(anchor);
        self.has_older = page.has_older;
        self.older_cursor = page.older_cursor;
        self.loading_earlier = false;
        self.queue_math_for_messages(0..self.messages.len(), cx);
        cx.notify();
    }

    pub(crate) fn new(
        composer: Entity<PromptComposer>,
        _window: &mut Window,
        _cx: &mut Context<'_, Self>,
    ) -> Self {
        Self {
            composer,
            conversation: None,
            messages: Vec::new(),
            list_state: ListState::new(0, ListAlignment::Top, LIST_OVERDRAW)
                .with_uniform_item_height(px(96.)),
            generation: 0,
            streaming_message: None,
            generation_task: None,
            generation_clock_task: None,
            generation_progress: None,
            older_cursor: None,
            has_older: false,
            loading_earlier: false,
            origins: HashMap::new(),
            math_cache: Arc::new(MathCache::default()),
            math_tasks: HashMap::new(),
        }
    }

    pub(crate) fn load(&mut self, thread: ConversationThread, cx: &mut Context<'_, Self>) {
        self.cancel_generation(cx);
        self.older_cursor = None;
        self.has_older = false;
        self.loading_earlier = false;
        self.origins.clear();
        self.conversation = Some(thread.conversation);
        self.messages = thread
            .messages
            .into_iter()
            .map(|message| Self::rendered_message(message, cx))
            .collect();
        self.queue_math_for_messages(0..self.messages.len(), cx);
        self.list_state
            .reset_with_uniform_height(self.messages.len(), px(96.));
        self.list_state.set_follow_mode(FollowMode::Normal);
        self.list_state.scroll_to_end();
        cx.notify();
    }

    pub(crate) fn clear(&mut self, cx: &mut Context<'_, Self>) {
        self.cancel_generation(cx);
        self.conversation = None;
        self.messages.clear();
        self.origins.clear();
        self.older_cursor = None;
        self.has_older = false;
        self.list_state.reset(0);
        cx.notify();
    }

    pub(crate) fn set_generation(
        &mut self,
        generation: GenerationConfig,
        cx: &mut Context<'_, Self>,
    ) {
        if let Some(conversation) = &mut self.conversation {
            conversation.generation = generation;
            cx.notify();
        }
    }

    pub(crate) fn rename(
        &mut self,
        id: magenta_core::ConversationId,
        title: String,
        cx: &mut Context<'_, Self>,
    ) {
        if let Some(conversation) = &mut self.conversation
            && conversation.id == id
        {
            conversation.title = title;
            cx.notify();
        }
    }

    #[cfg(test)]
    pub(crate) fn snapshot(&self) -> Option<ConversationThread> {
        Some(ConversationThread {
            conversation: self.conversation.clone()?,
            messages: self
                .messages
                .iter()
                .map(|message| message.message.clone())
                .collect(),
        })
    }

    pub(crate) const fn is_streaming(&self) -> bool {
        self.streaming_message.is_some()
    }

    pub(crate) fn start_generation(
        &mut self,
        user_message: Message,
        assistant_message: Message,
        provider_id: ProviderId,
        stream: GenerationStream,
        window: &Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.cancel_generation(cx);
        let user = Self::rendered_message(user_message, cx);
        if let Some(conversation) = &self.conversation {
            self.origins
                .insert(assistant_message.id, conversation.generation.clone());
        }
        let assistant = Self::rendered_message(assistant_message, cx);
        let old_count = self.messages.len();
        let assistant_id = assistant.message.id;
        self.messages.push(user);
        self.messages.push(assistant);
        self.list_state.splice(old_count..old_count, 2);
        self.list_state.set_follow_mode(FollowMode::Tail);
        self.list_state.scroll_to_end();
        self.begin_stream(assistant_id, provider_id, stream, window, cx);
        cx.notify();
    }

    pub(crate) fn regenerate(
        &mut self,
        assistant_id: MessageId,
        assistant_message: Message,
        provider_id: ProviderId,
        stream: GenerationStream,
        window: &Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(index) = self
            .messages
            .iter()
            .position(|message| message.message.id == assistant_id)
        else {
            return;
        };

        self.cancel_generation(cx);
        if let Some(conversation) = &self.conversation {
            self.origins
                .insert(assistant_message.id, conversation.generation.clone());
        }
        let new_assistant_id = assistant_message.id;
        self.messages[index] = Self::rendered_message(assistant_message, cx);
        self.list_state.remeasure_items(index..index + 1);
        self.list_state.set_follow_mode(FollowMode::Tail);
        self.list_state.scroll_to_end();
        self.begin_stream(new_assistant_id, provider_id, stream, window, cx);
        cx.notify();
    }

    pub(crate) fn cancel(&mut self, cx: &mut Context<'_, Self>) {
        self.cancel_generation(cx);
    }

    pub(crate) fn interrupt_for_shutdown(
        &mut self,
        cx: &mut ConversationContext<'_>,
    ) -> Option<Message> {
        let id = self.streaming_message.take()?;
        self.generation = self.generation.wrapping_add(1);
        self.generation_task.take();
        if let Some(progress) = self.clear_generation_progress().as_ref() {
            trace_generation_terminal(progress, "interrupted");
        }
        let message = self
            .messages
            .iter_mut()
            .find(|message| message.message.id == id)?;
        message.message.status = MessageStatus::Stopped;
        cx.notify();
        Some(message.message.clone())
    }

    pub(crate) fn request_regenerate(&self, message_id: MessageId, cx: &mut Context<'_, Self>) {
        if !self.is_streaming()
            && self
                .messages
                .iter()
                .any(|message| message.message.id == message_id)
        {
            cx.emit(ConversationViewEvent::Regenerate(message_id));
        }
    }

    fn rendered_message(message: Message, cx: &mut Context<'_, Self>) -> RenderedMessage {
        let (markdown, markdown_source, user_segments) = match message.role {
            MessageRole::Assistant => {
                let source = markdown::normalize_for_text_view(&message.content);
                (
                    Some(cx.new(|cx| TextViewState::markdown(&source, cx))),
                    Some(source),
                    Vec::new(),
                )
            }
            MessageRole::User => (
                None,
                None,
                code_fence::parse_segments(&message.content)
                    .into_iter()
                    .map(|segment| match segment {
                        ContentSegment::Text(text) => RenderedUserSegment::Text(text),
                        ContentSegment::Code(block) => {
                            let source_start = block.source_start;
                            let markdown = code_fence::markdown_for_block(&block);
                            RenderedUserSegment::Code {
                                source_start,
                                markdown: cx.new(|cx| TextViewState::markdown(&markdown, cx)),
                            }
                        }
                    })
                    .collect(),
            ),
        };
        RenderedMessage {
            message,
            markdown,
            markdown_source,
            user_segments,
        }
    }

    fn queue_math_for_messages(
        &mut self,
        indices: impl IntoIterator<Item = usize>,
        cx: &Context<'_, Self>,
    ) {
        for index in indices {
            let Some(message) = self.messages.get(index) else {
                continue;
            };
            if message.message.role != MessageRole::Assistant {
                continue;
            }
            for key in math::configured_formulas(&message.message.content, cx) {
                self.queue_math_render(key, cx);
            }
        }
    }

    fn queue_math_render(&mut self, key: FormulaKey, cx: &Context<'_, Self>) {
        if !self.math_cache.begin(key.clone()) {
            return;
        }

        let cache = self.math_cache.clone();
        let render_key = key.clone();
        let task_key = key.clone();
        self.math_tasks.insert(
            key,
            cx.spawn(async move |view, cx| {
                let result = cx
                    .background_spawn(async move { math::render_formula(&render_key) })
                    .await;
                _ = view.update(cx, |view, cx| {
                    cache.complete(task_key.clone(), result);
                    view.math_tasks.remove(&task_key);
                    view.refresh_math_messages(cx);
                });
            }),
        );
    }

    pub(crate) fn refresh_math_typography(&mut self, cx: &mut Context<'_, Self>) {
        self.math_tasks.clear();
        self.math_cache.clear();
        self.queue_math_for_messages(0..self.messages.len(), cx);
        self.refresh_math_messages(cx);
    }

    fn refresh_math_messages(&mut self, cx: &mut Context<'_, Self>) {
        for message in &mut self.messages {
            let (Some(markdown), Some(source)) = (&message.markdown, &message.markdown_source)
            else {
                continue;
            };
            markdown.update(cx, |state, cx| state.set_text(source, cx));
        }
        self.list_state.remeasure_items(0..self.messages.len());
        cx.notify();
    }

    fn begin_stream(
        &mut self,
        assistant_id: MessageId,
        provider_id: ProviderId,
        stream: GenerationStream,
        window: &Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        self.streaming_message = Some(assistant_id);
        self.generation_progress = Some(GenerationProgress::new(
            assistant_id,
            provider_id.clone(),
            self.origins.get(&assistant_id).cloned(),
        ));
        self.start_generation_clock(generation, assistant_id, window, cx);
        self.generation_task = Some(cx.spawn_in(window, async move |view, window| {
            let mut completed = None;
            let mut stream = stream;

            while let Some(event) = stream.next().await {
                match event {
                    Ok(GenerationEvent::Started) => {
                        if view
                            .update_in(window, |view, _, cx| {
                                view.mark_provider_started(generation, assistant_id, cx);
                            })
                            .is_err()
                        {
                            return;
                        }
                    }
                    Ok(GenerationEvent::TextDelta(chunk)) => {
                        if view
                            .update_in(window, |view, _, cx| {
                                view.push_stream_chunk(generation, assistant_id, &chunk, cx);
                            })
                            .is_err()
                        {
                            return;
                        }
                    }
                    Ok(GenerationEvent::Completed(outcome)) => {
                        completed = Some(outcome);
                        break;
                    }
                    Err(error) => {
                        _ = view.update_in(window, |view, window, cx| {
                            view.fail_stream(generation, assistant_id, error, window, cx);
                        });
                        return;
                    }
                }
            }

            if let Some(outcome) = completed {
                _ = view.update_in(window, |view, _, cx| {
                    view.finish_stream(generation, assistant_id, outcome, cx);
                });
            } else {
                let error = ProviderError::new(provider_id, IncompleteGeneration);
                _ = view.update_in(window, |view, window, cx| {
                    view.fail_stream(generation, assistant_id, error, window, cx);
                });
            }
        }));
        cx.emit(ConversationViewEvent::GenerationStarted);
    }

    fn start_generation_clock(
        &mut self,
        generation: u64,
        assistant_id: MessageId,
        window: &Window,
        cx: &Context<'_, Self>,
    ) {
        self.generation_clock_task.take();
        self.generation_clock_task = Some(cx.spawn_in(window, async move |view, window| {
            loop {
                window
                    .background_executor()
                    .timer(GENERATION_CLOCK_INTERVAL)
                    .await;

                match view.update_in(window, |view, _, cx| {
                    let active = view.generation == generation
                        && view.streaming_message == Some(assistant_id)
                        && view
                            .generation_progress
                            .as_ref()
                            .is_some_and(|progress| progress.message_id == assistant_id);
                    if active {
                        cx.notify();
                    }
                    active
                }) {
                    Ok(true) => {}
                    Ok(false) | Err(_) => break,
                }
            }
        }));
    }

    fn mark_provider_started(
        &mut self,
        generation: u64,
        assistant_id: MessageId,
        cx: &mut Context<'_, Self>,
    ) {
        if self.generation != generation || self.streaming_message != Some(assistant_id) {
            return;
        }

        let Some(progress) = self
            .generation_progress
            .as_mut()
            .filter(|progress| progress.message_id == assistant_id)
        else {
            return;
        };
        if progress.provider_started_at.is_none() {
            progress.provider_started_at = Some(Instant::now());
            progress.phase = GenerationPhase::Thinking;
            tracing::debug!(
                provider = %progress.provider.0,
                message_id = assistant_id.0,
                operation = "conversation.generate",
                "provider stream started"
            );
            cx.notify();
        }
    }

    fn push_stream_chunk(
        &mut self,
        generation: u64,
        assistant_id: MessageId,
        chunk: &str,
        cx: &mut Context<'_, Self>,
    ) {
        if self.generation != generation || self.streaming_message != Some(assistant_id) {
            return;
        }

        if let Some(progress) = self
            .generation_progress
            .as_mut()
            .filter(|progress| progress.message_id == assistant_id)
            && progress.first_text_at.is_none()
        {
            progress.first_text_at = Some(Instant::now());
            progress.phase = GenerationPhase::Responding;
        }

        let Some(index) = self
            .messages
            .iter()
            .position(|message| message.message.id == assistant_id)
        else {
            return;
        };
        let rendered = &mut self.messages[index];
        rendered.message.content.push_str(chunk);
        rendered.message.status = MessageStatus::Streaming;
        let normalized = markdown::normalize_for_text_view(&rendered.message.content);
        if let (Some(markdown), Some(previous)) = (
            rendered.markdown.as_ref(),
            rendered.markdown_source.as_ref(),
        ) {
            if let Some(delta) = normalized.strip_prefix(previous) {
                markdown.update(cx, |state, cx| state.push_str(delta, cx));
            } else {
                markdown.update(cx, |state, cx| state.set_text(&normalized, cx));
            }
        }
        rendered.markdown_source = Some(normalized);
        self.queue_math_for_messages([index], cx);
        self.list_state.remeasure_items(index..index + 1);
        cx.notify();
    }

    fn finish_stream(
        &mut self,
        generation: u64,
        assistant_id: MessageId,
        outcome: GenerationOutcome,
        cx: &mut Context<'_, Self>,
    ) {
        if self.generation != generation || self.streaming_message != Some(assistant_id) {
            return;
        }

        let progress = self.clear_generation_progress();
        if let Some(progress) = progress.as_ref() {
            trace_generation_terminal(progress, "completed");
        }

        if let Some(message) = self
            .messages
            .iter_mut()
            .find(|message| message.message.id == assistant_id)
        {
            message.message.status = MessageStatus::Complete;
            message.message.generation_outcome = Some(outcome);
        }
        self.streaming_message = None;
        if let Some(message) = self
            .messages
            .iter()
            .find(|item| item.message.id == assistant_id)
        {
            cx.emit(ConversationViewEvent::GenerationFinished(
                message.message.clone(),
            ));
        }
        cx.notify();
    }

    fn fail_stream(
        &mut self,
        generation: u64,
        assistant_id: MessageId,
        error: ProviderError,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.generation != generation || self.streaming_message != Some(assistant_id) {
            return;
        }

        let provider = error.provider.clone();
        let progress = self.clear_generation_progress();
        let model = progress
            .as_ref()
            .and_then(|progress| progress.configuration.as_ref())
            .map_or("unknown", |configuration| configuration.model.0.as_str());
        let effort = progress
            .as_ref()
            .and_then(|progress| progress.configuration.as_ref())
            .map_or("unknown", |configuration| configuration.effort.label());
        tracing::error!(
            error = ?error,
            provider = %provider.0,
            model,
            effort,
            phase = progress.as_ref().map_or("unknown", |progress| progress.phase.label()),
            elapsed_ms = progress.as_ref().map(|progress| duration_millis(progress.elapsed())),
            provider_started_ms = progress.as_ref().and_then(GenerationProgress::provider_started_ms),
            first_text_ms = progress.as_ref().and_then(GenerationProgress::first_text_ms),
            operation = "conversation.generate",
            "provider generation failed"
        );
        if let Some(message) = self
            .messages
            .iter_mut()
            .find(|message| message.message.id == assistant_id)
        {
            message.message.status = MessageStatus::Failed;
        }
        self.streaming_message = None;
        let application_error = MagentaError::ProviderGeneration {
            provider,
            source: error,
        };
        window.push_notification(notification_for_error(&application_error), cx);
        if let Some(message) = self
            .messages
            .iter()
            .find(|item| item.message.id == assistant_id)
        {
            cx.emit(ConversationViewEvent::GenerationFinished(
                message.message.clone(),
            ));
        }
        cx.notify();
    }

    fn cancel_generation(&mut self, cx: &mut Context<'_, Self>) {
        let Some(assistant_id) = self.streaming_message.take() else {
            self.generation_task.take();
            self.clear_generation_progress();
            return;
        };

        self.generation = self.generation.wrapping_add(1);
        self.generation_task.take();
        if let Some(progress) = self.clear_generation_progress().as_ref() {
            trace_generation_terminal(progress, "stopped");
        }
        if let Some(message) = self
            .messages
            .iter_mut()
            .find(|message| message.message.id == assistant_id)
        {
            message.message.status = MessageStatus::Stopped;
        }
        if let Some(message) = self
            .messages
            .iter()
            .find(|item| item.message.id == assistant_id)
        {
            cx.emit(ConversationViewEvent::GenerationFinished(
                message.message.clone(),
            ));
        }
        cx.notify();
    }

    fn clear_generation_progress(&mut self) -> Option<GenerationProgress> {
        self.generation_clock_task.take();
        self.generation_progress.take()
    }

    fn render_message(
        &self,
        index: usize,
        _window: &mut Window,
        cx: &App,
        view: Entity<Self>,
    ) -> AnyElement {
        let Some(message) = self.messages.get(index) else {
            return div().into_any_element();
        };

        let body = match message.message.role {
            MessageRole::User => Self::render_user_message(message, cx),
            MessageRole::Assistant => self.render_assistant_message(&message.message, cx, view),
        };

        div()
            .w_full()
            .px(px(24.))
            .py(px(8.))
            .child(
                div()
                    .w_full()
                    .max_w(MESSAGE_MAX_WIDTH)
                    .mx_auto()
                    .child(body),
            )
            .into_any_element()
    }

    fn render_user_message(message: &RenderedMessage, cx: &App) -> AnyElement {
        let actions = Clipboard::new(("copy-message", message.message.id.0))
            .value(message.message.content.clone())
            .tooltip("Copy message");
        let message_id = message.message.id.0;
        let style = TextViewStyle {
            paragraph_gap: rems(0.45),
            is_dark: cx.theme().is_dark(),
            ..Default::default()
        };
        let segments =
            message.user_segments.iter().enumerate().map(
                |(segment_index, segment)| match segment {
                    RenderedUserSegment::Text(text) => inline_code::render_plain_text(text, cx),
                    RenderedUserSegment::Code {
                        source_start,
                        markdown,
                    } => {
                        let code_id = message_id.wrapping_add(*source_start as u64);
                        let code_style = style.clone();
                        TextView::new(markdown)
                            .selectable(true)
                            .style(code_style)
                            .w_full()
                            .text_size(px(12.))
                            .line_height(px(18.))
                            .code_block_actions(move |code_block, _window, app| {
                                let code_id = code_id.wrapping_add(segment_index as u64);
                                let language = code_block.lang().unwrap_or_else(|| "Code".into());
                                h_flex()
                                    .items_center()
                                    .gap(px(5.))
                                    .child(
                                        div()
                                            .text_size(px(10.))
                                            .text_color(app.theme().muted_foreground)
                                            .child(language),
                                    )
                                    .child(
                                        Clipboard::new(("copy-user-code", code_id))
                                            .value(code_block.code())
                                            .tooltip("Copy code"),
                                    )
                            })
                            .into_any_element()
                    }
                },
            );

        div()
            .w_full()
            .flex()
            .flex_col()
            .items_end()
            .gap(px(5.))
            .child(
                div()
                    .max_w(USER_MESSAGE_MAX_WIDTH)
                    .px(px(14.))
                    .py(px(10.))
                    .rounded(px(12.))
                    .border_1()
                    .border_color(cx.theme().input.opacity(0.72))
                    .bg(cx.theme().secondary)
                    .text_size(px(13.))
                    .line_height(px(20.))
                    .text_color(cx.theme().foreground)
                    .child(v_flex().w_full().gap(px(8.)).children(segments)),
            )
            .child(h_flex().h(px(24.)).items_center().child(actions))
            .into_any_element()
    }

    fn render_assistant_header(&self, message: &Message, cx: &App) -> AnyElement {
        let model_label = self
            .origins
            .get(&message.id)
            .or_else(|| {
                self.conversation
                    .as_ref()
                    .map(|conversation| &conversation.generation)
            })
            .map_or_else(
                || "Model".to_owned(),
                |generation| generation.model.0.clone(),
            );
        let label = match message.status {
            MessageStatus::Stopped => format!("{model_label} · stopped"),
            MessageStatus::Failed => format!("{model_label} · failed"),
            MessageStatus::Complete | MessageStatus::Streaming => model_label,
        };

        h_flex()
            .h(px(24.))
            .items_center()
            .gap(px(8.))
            .child(
                Icon::new(IconName::Bot)
                    .xsmall()
                    .text_color(cx.theme().muted_foreground),
            )
            .child(
                div()
                    .text_size(px(11.))
                    .font_medium()
                    .text_color(cx.theme().muted_foreground)
                    .child(label),
            )
            .into_any_element()
    }

    fn render_assistant_message(
        &self,
        message: &Message,
        cx: &App,
        view: Entity<Self>,
    ) -> AnyElement {
        let Some(markdown) = self
            .messages
            .iter()
            .find(|candidate| candidate.message.id == message.id)
            .and_then(|candidate| candidate.markdown.as_ref())
        else {
            return div().into_any_element();
        };

        let style = TextViewStyle {
            paragraph_gap: rems(0.68),
            is_dark: cx.theme().is_dark(),
            ..Default::default()
        };
        let copy = Clipboard::new(("copy-message", message.id.0))
            .value(message.content.clone())
            .tooltip("Copy response");
        let regenerate_view = view.clone();
        let message_id = message.id;
        let code_id = message.id.0;
        let streaming = message.status == MessageStatus::Streaming;

        v_flex()
            .w_full()
            .gap(px(8.))
            .child(self.render_assistant_header(message, cx))
            .when(streaming, |this| {
                this.when_some(
                    self.render_generation_progress(message.id, cx, view),
                    gpui::ParentElement::child,
                )
            })
            .when(message.status == MessageStatus::Failed, |this| {
                this.child(
                    div()
                        .text_size(px(12.))
                        .text_color(cx.theme().muted_foreground)
                        .child("The response could not be generated. Try again."),
                )
            })
            .when(!message.content.is_empty(), |this| {
                this.child(
                    TextView::new(markdown)
                        .selectable(true)
                        .plugin(MarkdownMathPlugin::new(self.math_cache.clone()))
                        .plugin(MarkdownInlineCodePlugin)
                        .style(style)
                        .w_full()
                        .text_size(px(13.))
                        .line_height(px(21.))
                        .code_block_actions(move |code_block, _window, _cx| {
                            let code_id = code_block
                                .span
                                .map_or(code_id, |span| code_id.wrapping_add(span.start as u64));
                            Clipboard::new(("copy-code", code_id))
                                .value(code_block.code())
                                .tooltip("Copy code")
                        }),
                )
            })
            .when(!streaming, |this| {
                this.child(
                    h_flex()
                        .h(px(24.))
                        .items_center()
                        .gap(px(2.))
                        .child(copy)
                        .child(
                            Button::new(("regenerate-message", message_id.0))
                                .ghost()
                                .xsmall()
                                .icon(IconName::Redo2)
                                .tooltip("Regenerate response")
                                .accessibility_id(format!("regenerate-message-{}", message_id.0))
                                .on_click(move |_, _, cx| {
                                    regenerate_view.update(cx, |view, cx| {
                                        view.request_regenerate(message_id, cx);
                                    });
                                }),
                        ),
                )
            })
            .into_any_element()
    }

    fn render_generation_progress(
        &self,
        message_id: MessageId,
        cx: &App,
        view: Entity<Self>,
    ) -> Option<AnyElement> {
        let progress = self
            .generation_progress
            .as_ref()
            .filter(|progress| progress.message_id == message_id)?;
        let label = format!(
            "{} · {}",
            progress.phase.label(),
            format_elapsed(progress.elapsed())
        );

        Some(
            h_flex()
                .h(px(26.))
                .items_center()
                .gap(px(8.))
                .text_size(px(12.))
                .text_color(cx.theme().muted_foreground)
                .child(Icon::new(IconName::LoaderCircle).xsmall())
                .child(label)
                .child(
                    Button::new(("stop-response", message_id.0))
                        .ghost()
                        .xsmall()
                        .h(px(24.))
                        .px(px(6.))
                        .icon(Icon::empty().path("icons/generation-stop.svg"))
                        .label("Stop")
                        .tooltip("Stop response")
                        .accessibility_id(format!("stop-response-{}", message_id.0))
                        .on_click(move |_, _, cx| {
                            view.update(cx, |view, cx| {
                                view.cancel_generation(cx);
                            });
                        }),
                )
                .into_any_element(),
        )
    }
}

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
                    view.read(cx)
                        .render_message(index, window, cx, view.clone())
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
    }
}

#[cfg(test)]
mod tests {
    use std::{
        cell::RefCell,
        pin::Pin,
        rc::Rc,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
        task::{Context as TaskContext, Poll},
    };

    use futures_util::{Stream, stream};
    use gpui::{TestAppContext, size};
    use gpui_component::Root;
    use magenta_core::{
        ConversationId, EffortLevel, FinishReason, GenerationConfig, ModelId, TokenUsage,
    };

    use super::*;

    fn conversation() -> Conversation {
        Conversation {
            id: ConversationId::new(77),
            title: "Provider-ready stream".to_owned(),
            generation: GenerationConfig::new(
                ProviderId::new("demo"),
                ModelId::new("magenta-demo"),
                EffortLevel::Medium,
            ),
        }
    }

    fn stored_page(range: std::ops::Range<u64>) -> magenta_core::MessagePage {
        let has_older = range.start > 0;
        let messages = range
            .map(|id| magenta_core::StoredMessage {
                message: message(id, MessageRole::Assistant, MessageStatus::Complete),
                sequence: magenta_core::MessageSequence(i64::try_from(id).unwrap()),
                created_at: magenta_core::Timestamp(0),
                generation: conversation().generation,
            })
            .collect::<Vec<_>>();
        magenta_core::MessagePage {
            older_cursor: messages.first().map(|message| message.sequence),
            messages,
            has_older,
        }
    }

    #[test]
    fn elapsed_time_uses_compact_stable_units() {
        assert_eq!(format_elapsed(Duration::ZERO), "0s");
        assert_eq!(format_elapsed(Duration::from_secs(59)), "59s");
        assert_eq!(format_elapsed(Duration::from_secs(60)), "1m 00s");
        assert_eq!(format_elapsed(Duration::from_secs(3_725)), "1h 02m");
    }

    #[gpui::test]
    fn generation_progress_tracks_stream_phases_and_clears_on_stop(cx: &mut TestAppContext) {
        cx.update(gpui_component::init);
        let window = cx.open_window(size(px(900.), px(640.)), |window, cx| {
            let composer = cx.new(|cx| PromptComposer::new(window, cx));
            ConversationView::new(composer, window, cx)
        });

        window
            .update(cx, |view, window, cx| {
                view.load(
                    ConversationThread {
                        conversation: conversation(),
                        messages: Vec::new(),
                    },
                    cx,
                );
                view.start_generation(
                    message(1, MessageRole::User, MessageStatus::Complete),
                    message(2, MessageRole::Assistant, MessageStatus::Streaming),
                    ProviderId::new("demo"),
                    Box::pin(stream::pending()),
                    window,
                    cx,
                );

                let generation = view.generation;
                let progress = view
                    .generation_progress
                    .as_ref()
                    .expect("generation progress should start immediately");
                assert_eq!(progress.phase, GenerationPhase::Connecting);
                assert!(progress.provider_started_at.is_none());
                assert!(progress.first_text_at.is_none());

                view.mark_provider_started(generation, MessageId::new(2), cx);
                let progress = view.generation_progress.as_ref().unwrap();
                assert_eq!(progress.phase, GenerationPhase::Thinking);
                assert!(progress.provider_started_at.is_some());

                view.push_stream_chunk(generation, MessageId::new(2), "first", cx);
                let first_text_at = view
                    .generation_progress
                    .as_ref()
                    .and_then(|progress| progress.first_text_at)
                    .expect("the first text timestamp should be recorded");
                assert_eq!(
                    view.generation_progress.as_ref().unwrap().phase,
                    GenerationPhase::Responding
                );

                view.push_stream_chunk(generation, MessageId::new(2), " second", cx);
                assert_eq!(
                    view.generation_progress
                        .as_ref()
                        .and_then(|progress| progress.first_text_at),
                    Some(first_text_at)
                );

                view.cancel(cx);
                assert!(view.generation_progress.is_none());
                assert!(view.generation_clock_task.is_none());
            })
            .expect("the conversation test window should remain open");
    }

    #[gpui::test]
    fn prepending_history_preserves_visible_message_and_offset(cx: &mut TestAppContext) {
        cx.update(gpui_component::init);
        let window = cx.open_window(size(px(900.), px(640.)), |window, cx| {
            let composer = cx.new(|cx| PromptComposer::new(window, cx));
            ConversationView::new(composer, window, cx)
        });
        window
            .update(cx, |view, _, cx| {
                view.load_page(
                    magenta_core::ConversationPage {
                        conversation: conversation(),
                        page: stored_page(50..100),
                    },
                    cx,
                );
                view.list_state.scroll_to(gpui::ListOffset {
                    item_ix: 10,
                    offset_in_item: px(7.),
                });
                view.prepend_page(stored_page(0..50), cx);
                let anchor = view.list_state.logical_scroll_top();
                assert_eq!(anchor.item_ix, 60);
                assert_eq!(anchor.offset_in_item, px(7.));
                assert_eq!(view.messages[anchor.item_ix].message.id, MessageId(60));
                assert!(!view.has_older);
                view.prepend_page(stored_page(0..50), cx);
                assert_eq!(view.messages.len(), 100);
                assert_eq!(view.list_state.logical_scroll_top().item_ix, 60);
            })
            .unwrap();
    }

    #[gpui::test]
    fn stream_deltas_do_not_emit_persistence_events(cx: &mut TestAppContext) {
        cx.update(gpui_component::init);
        let saved = Rc::new(RefCell::new(Vec::new()));
        let events = Rc::clone(&saved);
        let window = cx.open_window(size(px(900.), px(640.)), |window, cx| {
            let composer = cx.new(|cx| PromptComposer::new(window, cx));
            ConversationView::new(composer, window, cx)
        });
        let subscription = window
            .update(cx, |view, window, cx| {
                let subscription = cx.subscribe(&cx.entity(), move |_, _, event, _| {
                    if let ConversationViewEvent::GenerationFinished(message) = event {
                        events.borrow_mut().push(message.clone());
                    }
                });
                view.load(
                    ConversationThread {
                        conversation: conversation(),
                        messages: Vec::new(),
                    },
                    cx,
                );
                view.start_generation(
                    message(1, MessageRole::User, MessageStatus::Complete),
                    message(2, MessageRole::Assistant, MessageStatus::Streaming),
                    ProviderId::new("test"),
                    Box::pin(stream::pending()),
                    window,
                    cx,
                );
                for _ in 0..100 {
                    view.push_stream_chunk(view.generation, MessageId(2), "x", cx);
                }
                subscription
            })
            .unwrap();
        cx.run_until_parked();
        assert!(saved.borrow().is_empty());
        window.update(cx, |view, _, cx| view.cancel(cx)).unwrap();
        cx.run_until_parked();
        assert_eq!(saved.borrow().len(), 1);
        assert_eq!(saved.borrow()[0].content.len(), 100);
        assert_eq!(saved.borrow()[0].status, MessageStatus::Stopped);
        drop(subscription);
    }

    fn message(id: u64, role: MessageRole, status: MessageStatus) -> Message {
        Message {
            id: MessageId::new(id),
            conversation_id: ConversationId::new(77),
            role,
            content: String::new(),
            status,
            attachments: Vec::new(),
            generation_outcome: None,
        }
    }

    struct DropAwareStream {
        dropped: Arc<AtomicBool>,
    }

    impl Stream for DropAwareStream {
        type Item = Result<GenerationEvent, ProviderError>;

        fn poll_next(self: Pin<&mut Self>, _cx: &mut TaskContext<'_>) -> Poll<Option<Self::Item>> {
            Poll::Pending
        }
    }

    impl Drop for DropAwareStream {
        fn drop(&mut self) {
            self.dropped.store(true, Ordering::SeqCst);
        }
    }

    #[gpui::test]
    fn loading_a_fixture_keeps_the_conversation_in_the_view(cx: &mut TestAppContext) {
        cx.update(gpui_component::init);
        let window = cx.open_window(size(px(900.), px(640.)), |window, cx| {
            let composer = cx.new(|cx| PromptComposer::new(window, cx));
            ConversationView::new(composer, window, cx)
        });

        window
            .update(cx, |view, _, cx| {
                let thread = ConversationThread {
                    conversation: conversation(),
                    messages: vec![message(1, MessageRole::User, MessageStatus::Complete)],
                };
                view.load(thread, cx);
                assert!(view.snapshot().is_some_and(|thread| {
                    thread.conversation.id == magenta_core::ConversationId::new(77)
                        && !thread.messages.is_empty()
                }));
            })
            .expect("the conversation test window should remain open");
    }

    #[gpui::test]
    fn user_messages_keep_prose_literal_and_isolate_fenced_code(cx: &mut TestAppContext) {
        cx.update(gpui_component::init);
        let window = cx.open_window(size(px(900.), px(640.)), |window, cx| {
            let composer = cx.new(|cx| PromptComposer::new(window, cx));
            ConversationView::new(composer, window, cx)
        });

        window
            .update(cx, |_, _, cx| {
                let message = Message {
                    id: MessageId::new(7),
                    conversation_id: magenta_core::ConversationId::new(3),
                    role: MessageRole::User,
                    content: "Before\n```rust\nlet answer = 42;\n```\nAfter".to_owned(),
                    status: MessageStatus::Complete,
                    attachments: Vec::new(),
                    generation_outcome: None,
                };
                let rendered = ConversationView::rendered_message(message, cx);

                assert!(matches!(
                    &rendered.user_segments[0],
                    RenderedUserSegment::Text(text) if text == "Before\n"
                ));
                assert!(matches!(
                    &rendered.user_segments[1],
                    RenderedUserSegment::Code {
                        source_start: 7,
                        ..
                    }
                ));
                assert!(matches!(
                    &rendered.user_segments[2],
                    RenderedUserSegment::Text(text) if text == "After"
                ));
            })
            .expect("the conversation test window should remain open");
    }

    #[gpui::test]
    fn completed_stream_stores_its_outcome(cx: &mut TestAppContext) {
        cx.update(gpui_component::init);
        let window = cx.open_window(size(px(900.), px(640.)), |window, cx| {
            let composer = cx.new(|cx| PromptComposer::new(window, cx));
            ConversationView::new(composer, window, cx)
        });
        let outcome = GenerationOutcome::new(
            FinishReason::Length,
            Some(TokenUsage {
                input_tokens: 12,
                output_tokens: 24,
            }),
        );

        window
            .update(cx, |view, window, cx| {
                view.load(
                    ConversationThread {
                        conversation: conversation(),
                        messages: Vec::new(),
                    },
                    cx,
                );
                view.start_generation(
                    message(1, MessageRole::User, MessageStatus::Complete),
                    message(2, MessageRole::Assistant, MessageStatus::Streaming),
                    ProviderId::new("demo"),
                    Box::pin(stream::iter([
                        Ok(GenerationEvent::Started),
                        Ok(GenerationEvent::TextDelta("hello λ".to_owned())),
                        Ok(GenerationEvent::Completed(outcome.clone())),
                    ])),
                    window,
                    cx,
                );
            })
            .expect("the conversation test window should remain open");
        cx.run_until_parked();

        window
            .update(cx, |view, _, _| {
                let thread = view
                    .snapshot()
                    .expect("the conversation should remain loaded");
                let assistant = thread
                    .messages
                    .last()
                    .expect("an assistant response should exist");
                assert_eq!(assistant.content, "hello λ");
                assert_eq!(assistant.status, MessageStatus::Complete);
                assert_eq!(assistant.generation_outcome, Some(outcome));
            })
            .expect("the conversation test window should remain open");
    }

    #[gpui::test]
    fn stream_ending_without_completion_marks_the_response_failed(cx: &mut TestAppContext) {
        cx.update(gpui_component::init);
        let view_slot = Rc::new(RefCell::new(None));
        let view_for_window = Rc::clone(&view_slot);
        let window = cx.open_window(size(px(900.), px(640.)), move |window, cx| {
            let composer = cx.new(|cx| PromptComposer::new(window, cx));
            let view = cx.new(|cx| ConversationView::new(composer, window, cx));
            view_for_window.replace(Some(view.clone()));
            Root::new(view, window, cx)
        });
        let view = view_slot
            .borrow()
            .clone()
            .expect("the conversation view should be created");

        window
            .update(cx, |_, window, cx| {
                view.update(cx, |view, cx| {
                    view.load(
                        ConversationThread {
                            conversation: conversation(),
                            messages: Vec::new(),
                        },
                        cx,
                    );
                    view.start_generation(
                        message(1, MessageRole::User, MessageStatus::Complete),
                        message(2, MessageRole::Assistant, MessageStatus::Streaming),
                        ProviderId::new("demo"),
                        Box::pin(stream::iter([Ok(GenerationEvent::Started)])),
                        window,
                        cx,
                    );
                });
            })
            .expect("the conversation test window should remain open");
        cx.run_until_parked();

        view.read_with(cx, |view, _| {
            let thread = view
                .snapshot()
                .expect("the conversation should remain loaded");
            assert_eq!(
                thread.messages.last().map(|message| message.status),
                Some(MessageStatus::Failed)
            );
        });
    }

    #[gpui::test]
    fn cancellation_drops_the_stream_and_rejects_stale_chunks(cx: &mut TestAppContext) {
        cx.update(gpui_component::init);
        let window = cx.open_window(size(px(900.), px(640.)), |window, cx| {
            let composer = cx.new(|cx| PromptComposer::new(window, cx));
            ConversationView::new(composer, window, cx)
        });
        let dropped = Arc::new(AtomicBool::new(false));

        window
            .update(cx, |view, window, cx| {
                view.load(
                    ConversationThread {
                        conversation: conversation(),
                        messages: Vec::new(),
                    },
                    cx,
                );
                view.start_generation(
                    message(1, MessageRole::User, MessageStatus::Complete),
                    message(2, MessageRole::Assistant, MessageStatus::Streaming),
                    ProviderId::new("demo"),
                    Box::pin(DropAwareStream {
                        dropped: Arc::clone(&dropped),
                    }),
                    window,
                    cx,
                );
                let stale_generation = view.generation;
                view.cancel(cx);
                view.push_stream_chunk(stale_generation, MessageId::new(2), "stale", cx);

                let thread = view
                    .snapshot()
                    .expect("the conversation should remain loaded");
                let assistant = thread
                    .messages
                    .last()
                    .expect("an assistant response should exist");
                assert_eq!(assistant.status, MessageStatus::Stopped);
                assert!(assistant.content.is_empty());
            })
            .expect("the conversation test window should remain open");

        cx.run_until_parked();
        assert!(dropped.load(Ordering::SeqCst));
    }

    #[gpui::test]
    fn superseding_a_generation_rejects_chunks_from_the_old_stream(cx: &mut TestAppContext) {
        cx.update(gpui_component::init);
        let window = cx.open_window(size(px(900.), px(640.)), |window, cx| {
            let composer = cx.new(|cx| PromptComposer::new(window, cx));
            ConversationView::new(composer, window, cx)
        });

        window
            .update(cx, |view, window, cx| {
                view.load(
                    ConversationThread {
                        conversation: conversation(),
                        messages: Vec::new(),
                    },
                    cx,
                );
                view.start_generation(
                    message(1, MessageRole::User, MessageStatus::Complete),
                    message(2, MessageRole::Assistant, MessageStatus::Streaming),
                    ProviderId::new("demo"),
                    Box::pin(stream::pending()),
                    window,
                    cx,
                );
                let stale_generation = view.generation;
                view.start_generation(
                    message(3, MessageRole::User, MessageStatus::Complete),
                    message(4, MessageRole::Assistant, MessageStatus::Streaming),
                    ProviderId::new("demo"),
                    Box::pin(stream::pending()),
                    window,
                    cx,
                );
                view.push_stream_chunk(stale_generation, MessageId::new(2), "stale", cx);

                let thread = view
                    .snapshot()
                    .expect("the conversation should remain loaded");
                let old_response = thread
                    .messages
                    .iter()
                    .find(|message| message.id == MessageId::new(2))
                    .expect("the superseded response should remain visible");
                assert_eq!(old_response.status, MessageStatus::Stopped);
                assert!(old_response.content.is_empty());
                view.cancel(cx);
            })
            .expect("the conversation test window should remain open");
    }
}
