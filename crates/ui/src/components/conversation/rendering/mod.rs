mod agent;
mod agent_cards;
mod assistant;

use chrono::Datelike as _;

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
        let day_label = self.message_day_label(index);

        div()
            .w_full()
            .px(px(24.))
            .py(px(13.))
            .child(
                v_flex()
                    .w_full()
                    .max_w(MESSAGE_MAX_WIDTH)
                    .mx_auto()
                    .gap(px(28.))
                    .when_some(day_label, |this, label| {
                        this.child(
                            div()
                                .w_full()
                                .text_center()
                                .text_size(px(12.))
                                .text_color(cx.theme().muted_foreground.opacity(0.82))
                                .child(label),
                        )
                    })
                    .child(body),
            )
            .into_any_element()
    }

    fn message_day_label(&self, index: usize) -> Option<String> {
        let message = self.messages.get(index)?;
        let current = chrono::DateTime::from_timestamp_millis(message.created_at.0)?
            .with_timezone(&chrono::Local);
        let starts_new_day = index == 0
            || self
                .messages
                .get(index.wrapping_sub(1))
                .is_none_or(|previous| {
                    chrono::DateTime::from_timestamp_millis(previous.created_at.0)
                        .map(|time| time.with_timezone(&chrono::Local).date_naive())
                        != Some(current.date_naive())
                });
        starts_new_day.then(|| format_message_day(current))
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
        let command_badge = message.message.command_id.as_ref().map(|command| {
            h_flex()
                .items_center()
                .gap(px(5.))
                .text_size(px(11.))
                .font_semibold()
                .text_color(cx.theme().primary)
                .child(format!("/{}", command.as_str()))
                .into_any_element()
        });
        let has_message_bubble =
            command_badge.is_some() || !message.message.content.trim().is_empty();

        div()
            .w_full()
            .flex()
            .flex_col()
            .items_end()
            .gap(px(5.))
            .when_some(attachments, gpui_kit::ParentElement::child)
            .when(has_message_bubble, |this| {
                this.child(
                    div()
                        .max_w(USER_MESSAGE_MAX_WIDTH)
                        .px(px(16.))
                        .py(px(11.))
                        .rounded(px(17.))
                        .border_1()
                        .border_color(cx.theme().border.opacity(0.38))
                        .bg(crate::components::visual::surface(
                            crate::components::visual::SurfaceLevel::Raised,
                            cx,
                        ))
                        .text_size(px(14.5))
                        .line_height(px(23.))
                        .text_color(cx.theme().foreground)
                        .child(
                            v_flex()
                                .w_full()
                                .gap(px(7.))
                                .when_some(command_badge, gpui_kit::ParentElement::child)
                                .when(!message.message.content.trim().is_empty(), |this| {
                                    this.children(segments)
                                }),
                        ),
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
        let tiles = message.attachments.iter().map(|attachment| {
            let path = attachment.path.clone();
            let name = attachment.name.clone();
            let preview_path = path.clone();
            let preview_name = name.clone();
            let preview_view = view.clone();

            Button::new(format!(
                "message-attachment-{}-{}",
                message.id.0,
                path.to_string_lossy()
            ))
            .ghost()
            .p_0()
            .relative()
            .size(if multiple { px(116.) } else { px(320.) })
            .h(if multiple { px(116.) } else { px(220.) })
            .overflow_hidden()
            .rounded(px(10.))
            .border_1()
            .border_color(cx.theme().input.opacity(0.72))
            .bg(cx.theme().secondary)
            .accessibility_label(format!("Open image {name}"))
            .tooltip(format!("Open {name}"))
            .focus_visible(|this| this.border_color(cx.theme().ring))
            .on_click(move |_, window, cx| {
                let return_focus = window.focused(cx);
                preview_view.update(cx, |conversation, cx| {
                    conversation.attachment_preview_return_focus = return_focus;
                    conversation.attachment_preview = Some(AttachmentPreview {
                        path: preview_path.clone(),
                        name: preview_name.clone(),
                    });
                    conversation.attachment_preview_focus.focus(window, cx);
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

    pub(super) fn close_attachment_preview(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.attachment_preview.take().is_some() {
            if let Some(return_focus) = self.attachment_preview_return_focus.take() {
                return_focus.focus(window, cx);
            }
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
                    cx.listener(|conversation, _: &CloseAttachmentPreview, window, cx| {
                        conversation.close_attachment_preview(window, cx);
                    }),
                )
                .child(
                    div()
                        .absolute()
                        .inset_0()
                        .bg(cx.theme().background.opacity(0.82))
                        .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                            backdrop_view.update(cx, |conversation, cx| {
                                conversation.close_attachment_preview(window, cx);
                            });
                        }),
                )
                .child(Self::attachment_preview_dialog(
                    preview_path,
                    preview_name,
                    close_view,
                    muted_foreground,
                    &self.attachment_preview_focus,
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
        focus_handle: &FocusHandle,
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
                    .track_focus(focus_handle)
                    .focus_trap("attachment-preview-trap", focus_handle)
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
                                    .on_click(move |_, window, cx| {
                                        view.update(cx, |conversation, cx| {
                                            conversation.close_attachment_preview(window, cx);
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
}

fn format_message_day(created_at: chrono::DateTime<chrono::Local>) -> String {
    let today = chrono::Local::now().date_naive();
    let date = created_at.date_naive();
    if date == today {
        return created_at.format("Today at %-H:%M").to_string();
    }
    if date == today - chrono::Days::new(1) {
        return created_at.format("Yesterday at %-H:%M").to_string();
    }
    if date.year() == today.year() {
        created_at.format("%a %-d %b at %-H:%M").to_string()
    } else {
        created_at.format("%-d %b %Y at %-H:%M").to_string()
    }
}
