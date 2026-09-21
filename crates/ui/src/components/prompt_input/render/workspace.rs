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
                .border_color(cx.theme().transparent)
                .bg(cx.theme().transparent)
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

    pub(super) fn workspace_rail(&self, view: Entity<Self>, cx: &Context<'_, Self>) -> AnyElement {
        h_flex()
            .w_full()
            .min_w_0()
            .min_h(px(40.))
            .items_center()
            .justify_between()
            .gap(px(8.))
            .px(px(10.))
            .mt(px(5.))
            .rounded(px(13.))
            .border_1()
            .border_color(cx.theme().foreground.opacity(0.065))
            .bg(super::super::super::visual::surface(
                super::super::super::visual::SurfaceLevel::Raised,
                cx,
            ))
            .child(self.workspace_controls(view, cx))
            .into_any_element()
    }

    pub(super) fn compact_chat_row(
        &self,
        view: Entity<Self>,
        generating: bool,
        ready: bool,
        can_add_attachment: bool,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        h_flex()
            .w_full()
            .min_w_0()
            .min_h(px(36.))
            .items_center()
            .gap(px(7.))
            .child(self.add_menu_button(view.clone(), can_add_attachment, cx))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(self.render_input_content(cx)),
            )
            .child(self.generation_controls(view, generating, ready, cx))
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
        let generation_controls =
            self.generation_controls(submit_view.clone(), generating, ready, cx);

        h_flex()
            .w_full()
            .min_h(px(30.))
            .items_center()
            .justify_between()
            .gap(px(10.))
            .child(self.add_menu_button(submit_view, can_add_attachment, cx))
            .child(generation_controls)
            .into_any_element()
    }

    fn add_menu_button(
        &self,
        view: Entity<Self>,
        can_add_attachment: bool,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let mode = self.mode.clone();
        let commands = self
            .command_descriptors()
            .into_iter()
            .filter(|command| command.supports_mode(&mode))
            .map(|command| {
                (
                    command.id.as_str().to_owned(),
                    command.label,
                    command.description,
                )
            })
            .collect::<Vec<_>>();
        let popover_id = if mode == ConversationMode::Agent {
            "prompt-add-menu-popover-work"
        } else {
            "prompt-add-menu-popover-chat"
        };

        let trigger = Button::new("prompt-add-menu")
            .ghost()
            .accessibility_id("prompt-add-menu")
            .debug_selector(|| "prompt-add-menu-trigger".into())
            .tooltip("Add context or choose a command")
            .size(px(32.))
            .p_0()
            .rounded_full()
            .border_1()
            .border_color(cx.theme().foreground.opacity(0.08))
            .bg(cx.theme().accent.opacity(0.38))
            .icon(IconName::Plus);

        Popover::new(popover_id)
            .anchor(Anchor::BottomLeft)
            .trigger(trigger)
            .appearance(false)
            .content(move |_, window, popover_cx| {
                Self::add_menu_content(
                    &view,
                    &mode,
                    can_add_attachment,
                    &commands,
                    window,
                    popover_cx,
                )
            })
            .into_any_element()
    }

    fn add_menu_content(
        view: &Entity<Self>,
        mode: &ConversationMode,
        can_add_attachment: bool,
        commands: &[(String, String, String)],
        window: &Window,
        cx: &Context<'_, PopoverState>,
    ) -> AnyElement {
        let popover = cx.entity();
        let mut content = v_flex()
            .debug_selector(|| "prompt-add-menu-surface".into())
            .w(px(328.))
            .max_h(px(390.))
            .overflow_hidden()
            .p(px(7.))
            .gap(px(3.))
            .rounded(px(14.))
            .border_1()
            .border_color(cx.theme().foreground.opacity(0.09))
            .bg(super::super::super::visual::surface(
                super::super::super::visual::SurfaceLevel::Floating,
                cx,
            ))
            .shadow(super::super::super::visual::floating_shadow(cx))
            .child(picker_heading("Add", cx));

        if can_add_attachment {
            let dismiss = popover.clone();
            content = content.child(
                Self::add_menu_row(
                    "prompt-add-images",
                    IconName::GalleryVerticalEnd,
                    "Images",
                    "Add up to four reference images",
                    cx,
                )
                .on_click(window.listener_for(
                    view,
                    move |composer, _, window, cx| {
                        dismiss.update(cx, |state, cx| state.dismiss(window, cx));
                        composer.choose_attachments(window, cx);
                    },
                )),
            );
        }

        if *mode == ConversationMode::Agent {
            let dismiss = popover.clone();
            content = content.child(
                Self::add_menu_row(
                    "prompt-choose-project-menu",
                    IconName::FolderOpen,
                    "Project",
                    "Choose the workspace for this chat",
                    cx,
                )
                .on_click(window.listener_for(
                    view,
                    move |composer, _, window, cx| {
                        dismiss.update(cx, |state, cx| state.dismiss(window, cx));
                        composer.choose_workspace(window, cx);
                    },
                )),
            );
        }

        if !commands.is_empty() {
            content = content.child(
                div()
                    .mx(px(7.))
                    .my(px(4.))
                    .h(px(1.))
                    .bg(cx.theme().border.opacity(0.72)),
            );
        }
        for (id, label, description) in commands {
            let command_id = id.clone();
            let dismiss = popover.clone();
            content = content.child(
                Self::add_menu_row(
                    format!("prompt-command-menu-{id}"),
                    IconName::SquareTerminal,
                    format!("/{id} · {label}"),
                    description.clone(),
                    cx,
                )
                .on_click(window.listener_for(
                    view,
                    move |composer, _, window, cx| {
                        dismiss.update(cx, |state, cx| state.dismiss(window, cx));
                        composer.select_command_by_id(&command_id, window, cx);
                    },
                )),
            );
        }
        content.into_any_element()
    }

    fn add_menu_row(
        id: impl Into<SharedString>,
        icon: IconName,
        label: impl Into<SharedString>,
        description: impl Into<SharedString>,
        cx: &App,
    ) -> Button {
        Button::new(id.into())
            .ghost()
            .w_full()
            .h(px(48.))
            .px(px(9.))
            .rounded(px(9.))
            .child(
                h_flex()
                    .w_full()
                    .min_w_0()
                    .items_center()
                    .gap(px(10.))
                    .child(
                        div()
                            .flex()
                            .flex_none()
                            .items_center()
                            .justify_center()
                            .size(px(28.))
                            .rounded(px(8.))
                            .bg(cx.theme().accent.opacity(0.62))
                            .text_color(cx.theme().muted_foreground)
                            .child(Icon::new(icon).xsmall()),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .items_start()
                            .gap(px(2.))
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .font_medium()
                                    .text_color(cx.theme().foreground)
                                    .child(label.into()),
                            )
                            .child(
                                div()
                                    .w_full()
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .text_size(px(10.))
                                    .text_color(cx.theme().muted_foreground)
                                    .child(description.into()),
                            ),
                    ),
            )
    }

    fn generation_controls(
        &self,
        submit_view: Entity<Self>,
        generating: bool,
        ready: bool,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
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
            )
            .into_any_element()
    }
}
