mod model;

use futures_util::StreamExt as _;
use gpui::{
    AnyElement, App, AppContext as _, Context, Entity, EventEmitter, FollowMode, IntoElement,
    ListAlignment, ListSizingBehavior, ListState, ParentElement as _, Render, Styled as _, Task,
    Window, div, linear_color_stop, linear_gradient, list, prelude::FluentBuilder as _, px, rems,
};
use gpui_component::{
    ActiveTheme as _, Icon, IconName, Sizable as _, StyledExt as _, WindowExt as _,
    button::{Button, ButtonVariants as _},
    clipboard::Clipboard,
    h_flex,
    text::{TextView, TextViewState, TextViewStyle},
    v_flex,
};
use magenta_core::{
    Conversation, GenerationConfig, GenerationEvent, GenerationStream, Message, MessageId,
    MessageRole, MessageStatus, ProviderError, ProviderId,
};

use crate::components::{
    code_fence::{self, ContentSegment},
    inline_code::{self, MarkdownInlineCodePlugin},
    prompt_input::PromptComposer,
};
use crate::{MagentaError, notification_for_error};

pub use model::{DemoCatalog, DemoThread, chat_model_for, summary_for};

const MESSAGE_MAX_WIDTH: gpui::Pixels = px(760.);
const USER_MESSAGE_MAX_WIDTH: gpui::Pixels = px(560.);
const LIST_OVERDRAW: gpui::Pixels = px(640.);

#[derive(Debug, thiserror::Error)]
#[error("provider stream ended before completion")]
struct IncompleteGeneration;

#[derive(Clone, Debug)]
pub enum ConversationViewEvent {
    GenerationStarted,
    GenerationFinished,
    Updated,
    Regenerate(MessageId),
}

struct RenderedMessage {
    message: Message,
    markdown: Option<Entity<TextViewState>>,
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
}

impl ConversationView {
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
        }
    }

    pub(crate) fn load(&mut self, thread: DemoThread, cx: &mut Context<'_, Self>) {
        self.cancel_generation(cx);
        self.conversation = Some(thread.conversation);
        self.messages = thread
            .messages
            .into_iter()
            .map(|message| Self::rendered_message(message, cx))
            .collect();
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

    pub(crate) fn snapshot(&self) -> Option<DemoThread> {
        Some(DemoThread {
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
        let (markdown, user_segments) = match message.role {
            MessageRole::Assistant => (
                Some(cx.new(|cx| TextViewState::markdown(&message.content, cx))),
                Vec::new(),
            ),
            MessageRole::User => (
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
            user_segments,
        }
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
        self.generation_task = Some(cx.spawn_in(window, async move |view, window| {
            let mut completed = false;
            let mut stream = stream;

            while let Some(event) = stream.next().await {
                match event {
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
                    Ok(GenerationEvent::Completed) => {
                        completed = true;
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

            if completed {
                _ = view.update_in(window, |view, _, cx| {
                    view.finish_stream(generation, assistant_id, cx);
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
        if let Some(markdown) = &rendered.markdown {
            markdown.update(cx, |state, cx| state.push_str(chunk, cx));
        }
        self.list_state.remeasure_items(index..index + 1);
        cx.emit(ConversationViewEvent::Updated);
        cx.notify();
    }

    fn finish_stream(
        &mut self,
        generation: u64,
        assistant_id: MessageId,
        cx: &mut Context<'_, Self>,
    ) {
        if self.generation != generation || self.streaming_message != Some(assistant_id) {
            return;
        }

        if let Some(message) = self
            .messages
            .iter_mut()
            .find(|message| message.message.id == assistant_id)
        {
            message.message.status = MessageStatus::Complete;
        }
        self.streaming_message = None;
        cx.emit(ConversationViewEvent::Updated);
        cx.emit(ConversationViewEvent::GenerationFinished);
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
        tracing::error!(
            error = ?error,
            provider = %provider.0,
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
        cx.emit(ConversationViewEvent::Updated);
        cx.emit(ConversationViewEvent::GenerationFinished);
        cx.notify();
    }

    fn cancel_generation(&mut self, cx: &mut Context<'_, Self>) {
        let Some(assistant_id) = self.streaming_message.take() else {
            self.generation_task.take();
            return;
        };

        self.generation = self.generation.wrapping_add(1);
        self.generation_task.take();
        if let Some(message) = self
            .messages
            .iter_mut()
            .find(|message| message.message.id == assistant_id)
        {
            message.message.status = MessageStatus::Stopped;
        }
        cx.emit(ConversationViewEvent::Updated);
        cx.emit(ConversationViewEvent::GenerationFinished);
        cx.notify();
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
                    .bg(linear_gradient(
                        160.,
                        linear_color_stop(cx.theme().button_hover.opacity(0.68), 0.),
                        linear_color_stop(cx.theme().button.opacity(0.82), 1.),
                    ))
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
            .conversation
            .as_ref()
            .map_or("Model", |conversation| chat_model_for(conversation).label());
        let label = match message.status {
            MessageStatus::Stopped => format!("{model_label} · stopped"),
            MessageStatus::Failed => format!("{model_label} · failed"),
            MessageStatus::Complete | MessageStatus::Streaming => model_label.to_owned(),
        };

        h_flex()
            .h(px(24.))
            .items_center()
            .gap(px(8.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(20.))
                    .rounded_full()
                    .bg(cx.theme().primary.opacity(0.16))
                    .text_color(cx.theme().primary)
                    .child(Icon::new(IconName::Bot).xsmall()),
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
        let regenerate_view = view;
        let message_id = message.id;
        let code_id = message.id.0;
        let streaming = message.status == MessageStatus::Streaming;

        v_flex()
            .w_full()
            .gap(px(8.))
            .child(self.render_assistant_header(message, cx))
            .when(message.content.is_empty() && streaming, |this| {
                this.child(
                    h_flex()
                        .gap(px(8.))
                        .text_size(px(12.))
                        .text_color(cx.theme().muted_foreground)
                        .child(Icon::new(IconName::LoaderCircle).xsmall())
                        .child("Magenta is thinking..."),
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
}

impl EventEmitter<ConversationViewEvent> for ConversationView {}

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
            .child(conversation_ambient_light(cx))
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
                    .relative()
                    .flex_none()
                    .w_full()
                    .px(px(24.))
                    .pt(px(10.))
                    .pb(px(18.))
                    .child(
                        div()
                            .absolute()
                            .top(px(-42.))
                            .left_0()
                            .right_0()
                            .h(px(48.))
                            .bg(linear_gradient(
                                180.,
                                linear_color_stop(cx.theme().background.opacity(0.), 0.),
                                linear_color_stop(cx.theme().background, 1.),
                            )),
                    )
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

fn conversation_ambient_light(cx: &Context<'_, ConversationView>) -> AnyElement {
    div()
        .absolute()
        .top(px(-170.))
        .right(px(-90.))
        .size(px(520.))
        .rounded_full()
        .bg(linear_gradient(
            145.,
            linear_color_stop(cx.theme().primary.opacity(0.08), 0.),
            linear_color_stop(cx.theme().background.opacity(0.), 1.),
        ))
        .opacity(0.68)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use gpui::{TestAppContext, size};

    use super::*;

    #[gpui::test]
    fn loading_a_fixture_keeps_the_conversation_in_the_view(cx: &mut TestAppContext) {
        cx.update(gpui_component::init);
        let window = cx.open_window(size(px(900.), px(640.)), |window, cx| {
            let composer = cx.new(|cx| PromptComposer::new(window, cx));
            ConversationView::new(composer, window, cx)
        });

        window
            .update(cx, |view, _, cx| {
                let thread = DemoCatalog::new()
                    .thread(magenta_core::ConversationId::new(1))
                    .cloned()
                    .expect("the first demo thread should exist");
                view.load(thread, cx);
                assert!(view.snapshot().is_some_and(|thread| {
                    thread.conversation.id == magenta_core::ConversationId::new(1)
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
}
