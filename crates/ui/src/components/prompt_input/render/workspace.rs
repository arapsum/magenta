use super::*;

impl PromptComposer {
    pub(crate) fn code_preview(&self, cx: &Context<'_, Self>) -> impl IntoElement {
        let preview = self.preview.clone();
        let style = TextViewStyle {
            paragraph_gap: rems(0.45),
            is_dark: cx.theme().is_dark(),
            ..Default::default()
        };
        let scrollable = self.preview_line_count > 8;

        v_flex()
            .id("prompt-code-preview")
            .flex_none()
            .w_full()
            .gap(px(5.))
            .border_t_1()
            .border_color(cx.theme().border.opacity(0.72))
            .pt(px(7.))
            .child(
                h_flex()
                    .h(px(16.))
                    .items_center()
                    .text_size(px(10.))
                    .text_color(cx.theme().muted_foreground)
                    .child("Code preview"),
            )
            .child(
                div()
                    .w_full()
                    .when(scrollable, |this| this.h(px(192.)))
                    .child(
                        TextView::new(&preview)
                            .selectable(true)
                            .scrollable(scrollable)
                            .style(style)
                            .w_full()
                            .text_size(px(12.))
                            .line_height(px(18.))
                            .code_block_actions(|code_block, _window, app| {
                                let code_id = code_block.span.map_or(0, |span| span.start);
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
                                        Clipboard::new(("copy-prompt-code", code_id))
                                            .value(code_block.code())
                                            .tooltip("Copy code"),
                                    )
                            }),
                    ),
            )
    }

    fn workspace_controls(&self, view: Entity<Self>, cx: &Context<'_, Self>) -> AnyElement {
        let files_view = view.clone();
        let changes_view = view.clone();
        let enabled = self.workspace_root.is_some();
        let utility_button = |id: &'static str, icon, label| {
            Button::new(id)
                .ghost()
                .compact()
                .h(px(30.))
                .px(px(8.))
                .rounded(px(8.))
                .border_1()
                .border_color(cx.theme().foreground.opacity(0.07))
                .bg(cx.theme().accent.opacity(0.34))
                .icon(icon)
                .label(label)
        };

        h_flex()
            .min_w_0()
            .items_center()
            .gap(px(6.))
            .child(self.workspace_button(view, cx))
            .child(
                utility_button("prompt-open-files", IconName::FolderOpen, "Files")
                    .disabled(!enabled)
                    .on_click(move |_, _, cx| {
                        files_view.update(cx, |composer, cx| {
                            composer.open_workspace_panel(PromptWorkspacePanel::Files, cx);
                        });
                    }),
            )
            .child(
                utility_button("prompt-open-changes", IconName::File, "Changes")
                    .disabled(!enabled)
                    .on_click(move |_, _, cx| {
                        changes_view.update(cx, |composer, cx| {
                            composer.open_workspace_panel(PromptWorkspacePanel::Changes, cx);
                        });
                    }),
            )
            .into_any_element()
    }

    pub(super) fn footer(
        &self,
        submit_view: Entity<Self>,
        generating: bool,
        ready: bool,
        can_add_attachment: bool,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        h_flex()
            .w_full()
            .min_h(px(30.))
            .items_center()
            .justify_between()
            .gap(px(10.))
            .child(
                h_flex()
                    .min_w_0()
                    .items_center()
                    .gap(px(6.))
                    .when(can_add_attachment, |this| {
                        this.child(Self::attachment_button(cx))
                    })
                    .when(self.mode == ConversationMode::Agent, |this| {
                        this.child(self.workspace_controls(submit_view.clone(), cx))
                    }),
            )
            .child(
                h_flex()
                    .min_w_0()
                    .items_center()
                    .gap(px(6.))
                    .child(self.model_selector(submit_view.clone(), cx))
                    .child(
                        Button::new("prompt-submit")
                            .when(generating, ButtonVariants::secondary)
                            .when(!generating, ButtonVariants::primary)
                            .disabled(!ready && !generating)
                            .accessibility_id(if generating {
                                "prompt-stop-response"
                            } else {
                                "prompt-submit"
                            })
                            .tooltip(if generating {
                                "Stop response"
                            } else if ready {
                                "Send message"
                            } else {
                                "Add a message before sending"
                            })
                            .size(px(36.))
                            .p_0()
                            .rounded_full()
                            .icon(if generating {
                                Icon::empty().path("icons/generation-stop.svg")
                            } else {
                                Icon::new(IconName::ChevronUp)
                            })
                            .on_click(move |_, _window, cx| {
                                submit_view.update(cx, |composer, cx| {
                                    if generating {
                                        composer.cancel(cx);
                                    } else {
                                        composer.submit(cx);
                                    }
                                });
                            }),
                    ),
            )
            .into_any_element()
    }
}
