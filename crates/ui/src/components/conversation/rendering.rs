mod agent;

use super::*;

impl ConversationView {
    pub(super) fn render_message(
        &self,
        index: usize,
        _window: &mut Window,
        cx: &App,
        view: &Entity<Self>,
    ) -> AnyElement {
        let Some(message) = self.messages.get(index) else {
            return div().into_any_element();
        };

        let body = match message.message.role {
            MessageRole::User => Self::render_user_message(message, view, cx),
            MessageRole::Assistant => self.render_assistant_message(message, cx, view),
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

    fn render_user_message(message: &RenderedMessage, view: &Entity<Self>, cx: &App) -> AnyElement {
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

        let attachments = Self::render_user_attachments(&message.message, view, cx);

        div()
            .w_full()
            .flex()
            .flex_col()
            .items_end()
            .gap(px(5.))
            .when_some(attachments, gpui_kit::ParentElement::child)
            .when(!message.message.content.trim().is_empty(), |this| {
                this.child(
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
            })
            .child(h_flex().h(px(24.)).items_center().child(actions))
            .into_any_element()
    }

    fn render_user_attachments(
        message: &Message,
        view: &Entity<Self>,
        cx: &App,
    ) -> Option<AnyElement> {
        if message.attachments.is_empty() {
            return None;
        }

        let multiple = message.attachments.len() > 1;
        let muted_foreground = cx.theme().muted_foreground;
        let tiles = message
            .attachments
            .iter()
            .enumerate()
            .map(|(index, attachment)| {
                let path = attachment.path.clone();
                let name = attachment.name.clone();
                let preview_path = path.clone();
                let preview_name = name;
                let preview_view = view.clone();

                div()
                    .id(format!("message-attachment-{}-{index}", message.id.0))
                    .relative()
                    .size(if multiple { px(116.) } else { px(320.) })
                    .h(if multiple { px(116.) } else { px(220.) })
                    .overflow_hidden()
                    .rounded(px(10.))
                    .border_1()
                    .border_color(cx.theme().input.opacity(0.72))
                    .bg(cx.theme().secondary)
                    .cursor_pointer()
                    .on_click(move |_, _, cx| {
                        preview_view.update(cx, |conversation, cx| {
                            conversation.attachment_preview = Some(AttachmentPreview {
                                path: preview_path.clone(),
                                name: preview_name.clone(),
                            });
                            cx.notify();
                        });
                    })
                    .child(
                        img(path)
                            .size_full()
                            .object_fit(ObjectFit::Cover)
                            .with_fallback(move || {
                                v_flex()
                                    .size_full()
                                    .items_center()
                                    .justify_center()
                                    .gap(px(5.))
                                    .text_size(px(11.))
                                    .text_color(muted_foreground)
                                    .child(Icon::new(IconName::GalleryVerticalEnd).small())
                                    .child("Image unavailable")
                                    .into_any_element()
                            }),
                    )
                    .into_any_element()
            });

        Some(
            div()
                .max_w(USER_MESSAGE_MAX_WIDTH)
                .flex()
                .flex_wrap()
                .justify_end()
                .gap(px(6.))
                .children(tiles)
                .into_any_element(),
        )
    }

    pub(super) fn close_attachment_preview(&mut self, cx: &mut Context<'_, Self>) {
        if self.attachment_preview.take().is_some() {
            cx.notify();
        }
    }

    pub(super) fn attachment_preview_overlay(&self, cx: &Context<'_, Self>) -> Option<AnyElement> {
        let preview = self.attachment_preview.as_ref()?;
        let view = cx.entity();
        let preview_path = preview.path.clone();
        let preview_name = preview.name.clone();
        let backdrop_view = view.clone();
        let close_view = view;
        let muted_foreground = cx.theme().muted_foreground;

        Some(
            div()
                .id("attachment-preview-overlay")
                .absolute()
                .inset_0()
                .key_context("AttachmentPreview")
                .on_action(
                    cx.listener(|conversation, _: &CloseAttachmentPreview, _, cx| {
                        conversation.close_attachment_preview(cx);
                    }),
                )
                .child(
                    div()
                        .absolute()
                        .inset_0()
                        .bg(cx.theme().background.opacity(0.82))
                        .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                            backdrop_view.update(cx, |conversation, cx| {
                                conversation.close_attachment_preview(cx);
                            });
                        }),
                )
                .child(Self::attachment_preview_dialog(
                    preview_path,
                    preview_name,
                    close_view,
                    muted_foreground,
                    cx,
                ))
                .into_any_element(),
        )
    }

    fn attachment_preview_dialog(
        path: PathBuf,
        name: String,
        view: Entity<Self>,
        muted_foreground: gpui_kit::Hsla,
        cx: &App,
    ) -> AnyElement {
        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .p(px(40.))
            .child(
                v_flex()
                    .id("attachment-preview-dialog")
                    .role(Role::Dialog)
                    .aria_label(format!("Image preview: {name}"))
                    .max_w(px(960.))
                    .max_h(px(720.))
                    .overflow_hidden()
                    .rounded(px(14.))
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().popover)
                    .shadow_lg()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(
                        h_flex()
                            .h(px(38.))
                            .justify_between()
                            .items_center()
                            .px(px(12.))
                            .border_b_1()
                            .border_color(cx.theme().border)
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(px(12.))
                                    .font_medium()
                                    .child(name),
                            )
                            .child(
                                Button::new("close-attachment-preview")
                                    .ghost()
                                    .small()
                                    .icon(IconName::Close)
                                    .tooltip("Close preview")
                                    .on_click(move |_, _, cx| {
                                        view.update(cx, |conversation, cx| {
                                            conversation.close_attachment_preview(cx);
                                        });
                                    }),
                            ),
                    )
                    .child(
                        img(path)
                            .w(px(880.))
                            .h(px(620.))
                            .object_fit(ObjectFit::Contain)
                            .with_fallback(move || {
                                v_flex()
                                    .w(px(560.))
                                    .h(px(300.))
                                    .items_center()
                                    .justify_center()
                                    .gap(px(8.))
                                    .text_color(muted_foreground)
                                    .child(Icon::new(IconName::GalleryVerticalEnd).small())
                                    .child("This image is no longer available")
                                    .into_any_element()
                            }),
                    ),
            )
            .into_any_element()
    }

    fn render_assistant_header(&self, message: &Message, cx: &App) -> AnyElement {
        let generation = self.origins.get(&message.id).or_else(|| {
            self.conversation
                .as_ref()
                .map(|conversation| &conversation.generation)
        });
        let model_label = generation.map_or_else(
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
                provider_icon(generation.map(|generation| &generation.provider))
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
            .when_some(
                Self::render_context_notice(rendered),
                gpui_kit::ParentElement::child,
            )
            .when(streaming, |this| {
                this.when_some(
                    self.render_generation_progress(message.id, cx, view),
                    gpui_kit::ParentElement::child,
                )
            })
            .when_some(
                self.render_agent_activities(message, cx, view),
                gpui_kit::ParentElement::child,
            )
            .when_some(
                self.render_agent_approval(message.id, cx, view),
                gpui_kit::ParentElement::child,
            )
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

    fn render_generation_progress(
        &self,
        message_id: MessageId,
        cx: &App,
        view: &Entity<Self>,
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

        let view = view.clone();
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
