use std::fmt::Write as _;

use chrono::Datelike as _;
use gpui_kit::component::accordion::Accordion;

use super::super::*;

impl ConversationView {
    fn render_assistant_header(&self, rendered: &RenderedMessage, cx: &App) -> AnyElement {
        let message = &rendered.message;
        let generation = self.origins.get(&message.id).or_else(|| {
            self.conversation
                .as_ref()
                .map(|conversation| &conversation.generation)
        });
        let model_label = generation.map_or_else(
            || "Model".to_owned(),
            |generation| display_model_name(&generation.model.0),
        );
        let label = match message.status {
            MessageStatus::Stopped => format!("{model_label} · stopped"),
            MessageStatus::Failed if self.has_successful_retry(message.id) => {
                format!("{model_label} · previous attempt")
            }
            MessageStatus::Failed => format!("{model_label} · failed"),
            MessageStatus::Complete | MessageStatus::Streaming => model_label,
        };

        let timestamp = relative_message_timestamp(rendered.created_at);

        h_flex()
            .h(px(28.))
            .items_center()
            .gap(px(9.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(26.))
                    .rounded_full()
                    .bg(cx.theme().primary.opacity(0.16))
                    .text_color(cx.theme().primary)
                    .child(
                        provider_icon(generation.map(|generation| &generation.provider)).xsmall(),
                    ),
            )
            .child(
                div()
                    .text_size(px(13.))
                    .font_semibold()
                    .text_color(cx.theme().foreground.opacity(0.82))
                    .child(label),
            )
            .when(!timestamp.is_empty(), |this| {
                this.child(
                    div()
                        .size(px(3.))
                        .rounded_full()
                        .bg(cx.theme().muted_foreground.opacity(0.55)),
                )
                .child(
                    div()
                        .text_size(px(12.))
                        .font_medium()
                        .text_color(cx.theme().muted_foreground.opacity(0.82))
                        .child(timestamp),
                )
            })
            .into_any_element()
    }

    #[allow(clippy::too_many_lines)]
    pub(super) fn render_assistant_message(
        &self,
        rendered: &RenderedMessage,
        cx: &App,
        view: &Entity<Self>,
    ) -> AnyElement {
        let message = &rendered.message;
        let Some(markdown) = self
            .messages
            .iter()
            .find(|candidate| candidate.message.id == message.id)
            .and_then(|candidate| candidate.markdown.as_ref())
        else {
            return div().into_any_element();
        };

        let style = conversation_text_style(cx);
        let copy = Clipboard::new(("copy-message", message.id.0))
            .value(message.content.clone())
            .tooltip("Copy response");
        let regenerate_view = view.clone();
        let message_id = message.id;
        let code_id = message.id.0;
        let streaming = message.status == MessageStatus::Streaming;
        let superseded_failure = self.has_successful_retry(message_id);
        let action_label = if message.status == MessageStatus::Failed {
            "Retry response"
        } else {
            "Regenerate response"
        };

        v_flex()
            .w_full()
            .gap(px(15.))
            .child(self.render_assistant_header(rendered, cx))
            .when_some(
                Self::render_context_notice(rendered),
                gpui_kit::ParentElement::child,
            )
            .child(self.render_assistant_trace(message, cx, view))
            .when_some(
                self.render_agent_approval(message.id, cx, view),
                gpui_kit::ParentElement::child,
            )
            .when_some(
                self.render_workspace_review(message.id, cx, view),
                gpui_kit::ParentElement::child,
            )
            .when_some(
                message
                    .failure
                    .as_ref()
                    .filter(|_| !superseded_failure)
                    .map(|failure| Self::render_generation_failure(message, failure, cx, view)),
                gpui_kit::ParentElement::child,
            )
            .when(
                message.status == MessageStatus::Failed
                    && message.failure.is_none()
                    && !superseded_failure,
                |this| {
                    this.child(
                        div()
                            .text_size(px(12.))
                            .text_color(cx.theme().muted_foreground)
                            .child("The response could not be generated. Try again."),
                    )
                },
            )
            .when(!message.content.is_empty(), |this| {
                this.child(
                    TextView::new(markdown)
                        .selectable(true)
                        .plugin(MarkdownMathPlugin::new(self.math_cache.clone()))
                        .plugin(PremiumStepHeadingPlugin)
                        .plugin(MarkdownInlineCodePlugin)
                        .plugin(PremiumOrderedListPlugin::new(self.math_cache.clone()))
                        .plugin(PremiumCodeBlockPlugin)
                        .style(style)
                        .w_full()
                        .text_size(px(16.))
                        .line_height(px(25.))
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
            .when(
                !streaming
                    && !superseded_failure
                    && (message.status != MessageStatus::Failed || message.failure.is_none()),
                |this| {
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
                                    .tooltip(action_label)
                                    .accessibility_id(format!(
                                        "regenerate-message-{}",
                                        message_id.0
                                    ))
                                    .on_click(move |_, _, cx| {
                                        regenerate_view.update(cx, |view, cx| {
                                            view.request_regenerate(message_id, cx);
                                        });
                                    }),
                            ),
                    )
                },
            )
            .into_any_element()
    }

    fn has_successful_retry(&self, message_id: MessageId) -> bool {
        let Some(index) = self
            .messages
            .iter()
            .position(|candidate| candidate.message.id == message_id)
        else {
            return false;
        };

        self.messages[index + 1..]
            .iter()
            .take_while(|candidate| candidate.message.role != MessageRole::User)
            .any(|candidate| {
                candidate.message.role == MessageRole::Assistant
                    && candidate.message.status == MessageStatus::Complete
            })
    }

    #[allow(clippy::too_many_lines)]
    fn render_generation_failure(
        message: &Message,
        failure: &magenta_core::MessageFailure,
        cx: &App,
        view: &Entity<Self>,
    ) -> AnyElement {
        let has_side_effects = message.assistant_trace.entries.iter().any(|entry| {
            entry.kind == AssistantTraceKind::Tool
                && matches!(
                    entry.tool_name.as_deref(),
                    Some("create_file" | "apply_patch" | "run_command")
                )
        });
        let (title, explanation, primary_label, secondary_label) = match failure.category {
            magenta_core::MessageFailureCategory::Authentication => (
                "Sign in required",
                "Reconnect the provider account before trying this response again.",
                "Open provider settings",
                None,
            ),
            magenta_core::MessageFailureCategory::Permission => (
                "Model access unavailable",
                "This account cannot use the selected model. Choose another model to continue.",
                "Choose another model",
                None,
            ),
            magenta_core::MessageFailureCategory::RateLimit => (
                "Rate limit reached",
                "The provider temporarily limited this request. Try again in a moment.",
                "Retry",
                None,
            ),
            magenta_core::MessageFailureCategory::Connection => (
                "Connection interrupted",
                "The provider connection was interrupted before the response finished.",
                "Retry",
                None,
            ),
            magenta_core::MessageFailureCategory::Service => (
                "Provider unavailable",
                "The selected provider is having trouble. Try the response again.",
                "Retry",
                None,
            ),
            magenta_core::MessageFailureCategory::InvalidRequest => (
                "Request needs adjustment",
                "Change the request, then try again.",
                "Edit request",
                None,
            ),
            magenta_core::MessageFailureCategory::Context => (
                "Context limit reached",
                "This request is larger than the selected model can accept. Remove some context, then try again.",
                "Edit request",
                None,
            ),
            magenta_core::MessageFailureCategory::AgentLimit
            | magenta_core::MessageFailureCategory::RepeatedToolCalls => (
                if failure.category == magenta_core::MessageFailureCategory::AgentLimit {
                    "Out of limits"
                } else {
                    "Agent stopped safely"
                },
                if failure.category == magenta_core::MessageFailureCategory::AgentLimit {
                    "The workspace agent reached its safety ceiling before finishing. Continue from the completed work."
                } else {
                    "The workspace agent repeated actions without making progress. Continue from the completed work."
                },
                if has_side_effects {
                    "Prepare continuation"
                } else {
                    "Retry"
                },
                None,
            ),
            magenta_core::MessageFailureCategory::IncompleteResponse => (
                "Response incomplete",
                "The provider ended the response before it was complete.",
                "Retry",
                None,
            ),
            magenta_core::MessageFailureCategory::Unknown => (
                "Response could not be generated",
                "The selected model could not finish this response. Try again or choose another model.",
                "Retry",
                Some("Choose another model"),
            ),
        };
        let message_id = message.id;
        let primary_view = view.clone();
        let primary_event = match failure.category {
            magenta_core::MessageFailureCategory::Authentication => {
                ConversationViewEvent::OpenProviderSettings
            }
            magenta_core::MessageFailureCategory::Permission => {
                ConversationViewEvent::ChooseModelForRetry(message_id)
            }
            magenta_core::MessageFailureCategory::InvalidRequest
            | magenta_core::MessageFailureCategory::Context => ConversationViewEvent::FocusComposer,
            magenta_core::MessageFailureCategory::AgentLimit
            | magenta_core::MessageFailureCategory::RepeatedToolCalls
                if has_side_effects =>
            {
                ConversationViewEvent::PrepareContinue(message_id)
            }
            _ => ConversationViewEvent::Retry(message_id),
        };
        let mut technical = format!(
            "Reference: {}\nProvider: {}",
            failure.reference_code, failure.provider.0
        );
        if let Some(detail) = failure.detail {
            match detail {
                magenta_core::MessageFailureDetail::HttpStatus { status } => {
                    let _ = write!(technical, "\nHTTP status: {status}");
                }
                magenta_core::MessageFailureDetail::AgentLimits {
                    observed_rounds,
                    permitted_rounds,
                    observed_tool_calls,
                    permitted_tool_calls,
                } => {
                    let _ = write!(
                        technical,
                        "\nRounds: {observed_rounds}/{permitted_rounds}\nTool calls: {observed_tool_calls}/{permitted_tool_calls}"
                    );
                }
            }
        }
        let copy = Clipboard::new(("copy-failure-diagnostics", message_id.0))
            .value(technical.clone())
            .tooltip("Copy diagnostics");
        let mut details = Accordion::new(("generation-failure-details", message_id.0))
            .multiple(false)
            .bordered(false)
            .small();
        details = details.item(|item| {
            item.open(false).title("Technical details").child(
                v_flex()
                    .gap(px(6.))
                    .text_size(px(11.))
                    .text_color(cx.theme().muted_foreground)
                    .child(technical)
                    .child(copy),
            )
        });
        v_flex()
            .w_full()
            .gap(px(8.))
            .p(px(12.))
            .rounded(px(10.))
            .border_1()
            .border_color(cx.theme().danger.opacity(0.55))
            .bg(cx.theme().danger.opacity(0.08))
            .child(
                h_flex()
                    .items_center()
                    .gap(px(7.))
                    .child(Icon::new(IconName::CircleX).small())
                    .child(
                        v_flex()
                            .gap(px(2.))
                            .child(div().font_medium().child(title))
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(cx.theme().muted_foreground)
                                    .child(explanation),
                            ),
                    ),
            )
            .when(!message.content.is_empty(), |this| {
                this.child(
                    div()
                        .text_size(px(11.))
                        .text_color(cx.theme().muted_foreground)
                        .child("Partial response preserved below"),
                )
            })
            .child(
                h_flex()
                    .items_center()
                    .gap(px(6.))
                    .child(
                        Button::new(("generation-failure-primary", message_id.0))
                            .primary()
                            .small()
                            .label(primary_label)
                            .accessibility_id(format!(
                                "generation-failure-primary-{}",
                                message_id.0
                            ))
                            .on_click(move |_, _, cx| {
                                primary_view.update(cx, |_, cx| cx.emit(primary_event.clone()));
                            }),
                    )
                    .when_some(secondary_label, |this, label| {
                        let secondary_view = view.clone();
                        this.child(
                            Button::new(("generation-failure-secondary", message_id.0))
                                .ghost()
                                .small()
                                .label(label)
                                .on_click(move |_, _, cx| {
                                    secondary_view.update(cx, |view, cx| {
                                        view.request_choose_model(message_id, cx);
                                    });
                                }),
                        )
                    }),
            )
            .child(details)
            .into_any_element()
    }

    fn render_context_notice(rendered: &RenderedMessage) -> Option<AnyElement> {
        (rendered.omitted_context_messages > 0).then(|| {
            Button::new(("context-trimmed", rendered.message.id.0))
                .ghost()
                .xsmall()
                .label(format!(
                    "Context trimmed · {} earlier messages omitted",
                    rendered.omitted_context_messages
                ))
                .tooltip(
                    "The model received the newest complete turns that fit its context window.",
                )
                .into_any_element()
        })
    }
}

fn relative_message_timestamp(timestamp: magenta_core::Timestamp) -> String {
    let Some(created_at) = chrono::DateTime::from_timestamp_millis(timestamp.0)
        .map(|time| time.with_timezone(&chrono::Local))
    else {
        return String::new();
    };
    let now = chrono::Local::now();
    let elapsed = now.signed_duration_since(created_at);
    let minutes = elapsed.num_minutes().max(0);
    match minutes {
        0 => "now".to_owned(),
        1..=59 => format!("{minutes}m ago"),
        60..=1_439 => format!("{}h ago", minutes / 60),
        1_440..=10_079 => format!("{}d ago", minutes / 1_440),
        _ if created_at.year() == now.year() => created_at.format("%-d %b").to_string(),
        _ => created_at.format("%-d %b %Y").to_string(),
    }
}

fn display_model_name(model: &str) -> String {
    model.get(..3).map_or_else(
        || model.to_owned(),
        |prefix| {
            if prefix.eq_ignore_ascii_case("gpt") {
                format!("GPT{}", &model[3..])
            } else {
                model.to_owned()
            }
        },
    )
}

#[cfg(test)]
#[path = "../../../../test/components/conversation/rendering/assistant.rs"]
mod timestamp_tests;
