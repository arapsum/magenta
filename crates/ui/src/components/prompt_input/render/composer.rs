use super::*;

impl PromptComposer {
    pub(super) fn render_input_content(&self, cx: &Context<'_, Self>) -> AnyElement {
        let view = cx.entity();

        v_flex()
            .debug_selector(|| "prompt-input-content".into())
            .flex_1()
            .min_h_0()
            .gap(px(7.))
            .when_some(self.inline_error(), |this, error| {
                this.child(
                    h_flex()
                        .items_start()
                        .gap(px(6.))
                        .text_size(px(12.))
                        .text_color(cx.theme().danger)
                        .child(Icon::new(IconName::CircleX).xsmall())
                        .child(
                            v_flex()
                                .gap(px(2.))
                                .child(div().font_medium().child(error.title))
                                .child(div().child(error.message)),
                        ),
                )
            })
            .when_some(self.blocking_error(), |this, error| {
                this.child(
                    h_flex()
                        .items_start()
                        .gap(px(6.))
                        .text_size(px(12.))
                        .text_color(cx.theme().warning)
                        .child(Icon::new(IconName::CircleX).xsmall())
                        .child(
                            v_flex()
                                .gap(px(2.))
                                .child(div().font_medium().child(error.title))
                                .child(div().child(error.message)),
                        ),
                )
            })
            .when(self.is_model_retry(), |this| {
                this.child(
                    div()
                        .text_size(px(12.))
                        .text_color(cx.theme().muted_foreground)
                        .child(
                            "Choose a model above, then press Retry to try this response again.",
                        ),
                )
            })
            .when(self.selected_command.is_some(), |this| {
                this.child(h_flex().w_full().child(self.command_chip(&view, cx)))
            })
            .when(!self.attachments.is_empty(), |this| {
                this.child(self.attachment_strip(cx))
            })
            .when(!self.preview_source.is_empty(), |this| {
                this.child(self.code_preview(cx))
            })
            .child(
                Textarea::new(&self.input)
                    .appearance(false)
                    .bordered(false)
                    .aria_label("Chat message composer")
                    .w_full()
                    .flex_1()
                    .min_h(px(30.))
                    .p_0()
                    .text_size(px(15.))
                    .line_height(px(22.)),
            )
            .into_any_element()
    }

    pub(crate) fn mode_selector(&self, view: Entity<Self>, cx: &App) -> AnyElement {
        let mode = self.mode.clone();
        let agent_available = self.agent_capability.available();

        div()
            .debug_selector(|| "prompt-mode-selector".into())
            .rounded_full()
            .border_1()
            .border_color(cx.theme().foreground.opacity(0.07))
            .bg(super::super::super::visual::surface(
                super::super::super::visual::SurfaceLevel::Recessed,
                cx,
            ))
            .p(px(2.))
            .child(
                ToggleGroup::new("prompt-mode-selector")
                    .segmented()
                    .with_size(gpui_kit::component::Size::Small)
                    .child(
                        Toggle::new("prompt-mode-chat")
                            .label("Chat")
                            .w(px(60.))
                            .h(px(28.))
                            .checked(mode == ConversationMode::Chat),
                    )
                    .child(
                        Toggle::new("prompt-mode-work")
                            .label("Work")
                            .w(px(60.))
                            .h(px(28.))
                            .checked(mode == ConversationMode::Agent)
                            .disabled(!agent_available),
                    )
                    .on_click(move |checks, window, cx| {
                        let selects_work = super::super::super::select_second_segment(
                            checks,
                            mode == ConversationMode::Agent,
                        );
                        let next = if selects_work {
                            ConversationMode::Agent
                        } else {
                            ConversationMode::Chat
                        };

                        view.update(cx, |composer, cx| {
                            composer.select_mode(next, window, cx);
                        });
                    }),
            )
            .into_any_element()
    }

    pub(crate) fn workspace_button(
        &self,
        view: Entity<Self>,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let label = self
            .workspace_root
            .as_ref()
            .and_then(|path| path.file_name())
            .and_then(|name| name.to_str())
            .map_or_else(|| "Choose project".to_owned(), ToOwned::to_owned);
        let button = Button::new("prompt-workspace")
            .ghost()
            .compact()
            .h(px(30.))
            .px(px(8.))
            .rounded(px(8.))
            .border_1()
            .border_color(cx.theme().transparent)
            .bg(cx.theme().transparent)
            .icon(IconName::FolderOpen)
            .label(label)
            .tooltip(if self.agent_capability.commands() {
                "Choose the project this agent can access"
            } else {
                "Choose a project. Sandboxed commands are unavailable; file tools still work"
            })
            .on_click(move |_, window, cx| {
                view.update(cx, |composer, cx| composer.choose_workspace(window, cx));
            });
        button.into_any_element()
    }

    fn attachment_strip(&self, cx: &Context<'_, Self>) -> impl IntoElement {
        let view = cx.entity();

        h_flex()
            .h(px(36.))
            .gap(px(7.))
            .children(self.attachments.iter().map(|attachment| {
                let attachment_id = attachment.id;
                let remove_path = attachment.path.clone();
                let image_path = attachment.path.clone();
                let name = attachment.name.clone();
                let remove_view = view.clone();

                div()
                    .id(("prompt-attachment", attachment_id))
                    .relative()
                    .size(px(34.))
                    .overflow_hidden()
                    .rounded(px(7.))
                    .border_1()
                    .border_color(cx.theme().border)
                    .child(
                        img(image_path)
                            .size_full()
                            .object_fit(ObjectFit::Cover)
                            .with_fallback(|| {
                                div()
                                    .size_full()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child(Icon::new(IconName::GalleryVerticalEnd).xsmall())
                                    .into_any_element()
                            }),
                    )
                    .child(
                        Button::new(("remove-prompt-attachment", attachment_id))
                            .ghost()
                            .accessibility_id(format!("remove-attachment-{name}"))
                            .tooltip(format!("Remove {name}"))
                            .absolute()
                            .top(px(1.))
                            .right(px(1.))
                            .size(px(15.))
                            .p_0()
                            .rounded_full()
                            .bg(cx.theme().background.opacity(0.86))
                            .icon(IconName::CircleX)
                            .on_click(move |_, _, cx| {
                                remove_view.update(cx, |composer, cx| {
                                    composer.remove_attachment(&remove_path, cx);
                                });
                            }),
                    )
            }))
    }
}
