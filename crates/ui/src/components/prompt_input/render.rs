use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _, StyledExt as _, box_shadow,
    button::{Button, ButtonVariants, Toggle, ToggleGroup},
    clipboard::Clipboard,
    h_flex,
    input::Textarea,
    popover::{Popover, PopoverState},
    text::{TextView, TextViewStyle},
    v_flex,
};
use gpui_kit::{
    Anchor, AnyElement, App, Context, Entity, Focusable as _, InteractiveElement as _, IntoElement,
    ObjectFit, ParentElement as _, Render, SharedString, Styled as _, StyledImage as _, Window,
    div, img, linear_color_stop, linear_gradient, prelude::FluentBuilder as _, px, rems,
};
use magenta_core::{ConversationMode, EffortLevel, ModelDescriptor, ModelId, ProviderId};

use crate::components::provider_icon;

use super::{MAX_ATTACHMENTS, PromptComposer, PromptWorkspacePanel};

impl PromptComposer {
    fn render_chat_content(
        &self,
        submit_view: Entity<Self>,
        generating: bool,
        ready: bool,
        can_add_attachment: bool,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let submit = submit_view.clone();

        v_flex()
            .w_full()
            .gap(px(8.))
            .when_some(self.inline_error(), |this, error| {
                this.child(
                    div()
                        .text_size(px(12.))
                        .text_color(cx.theme().danger)
                        .child(format!("{} {}", error.title, error.message)),
                )
            })
            .when_some(self.blocking_error(), |this, error| {
                this.child(
                    div()
                        .text_size(px(12.))
                        .text_color(cx.theme().warning)
                        .child(format!("{} {}", error.title, error.message)),
                )
            })
            .when(!self.attachments.is_empty(), |this| {
                this.child(self.attachment_strip(cx))
            })
            .child(
                h_flex()
                    .w_full()
                    .items_center()
                    .gap(px(8.))
                    .when(can_add_attachment, |this| {
                        this.child(Self::attachment_button(cx))
                    })
                    .child(
                        Textarea::new(&self.input)
                            .appearance(false)
                            .bordered(false)
                            .aria_label("Chat message composer")
                            .flex_1()
                            .min_w_0()
                            .min_h(px(32.))
                            .p_0()
                            .text_size(px(15.))
                            .line_height(px(22.)),
                    )
                    .child(self.model_selector(submit_view, cx))
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
                            .on_click(move |_, _, cx| {
                                submit.update(cx, |composer, cx| {
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

    fn render_input_content(&self, cx: &Context<'_, Self>) -> AnyElement {
        v_flex()
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

    fn mode_selector(&self, view: Entity<Self>, cx: &Context<'_, Self>) -> AnyElement {
        let mode = self.mode.clone();
        let agent_available = self.agent_capability.available();

        div()
            .debug_selector(|| "prompt-mode-selector".into())
            .rounded_full()
            .border_1()
            .border_color(cx.theme().foreground.opacity(0.07))
            .bg(super::super::visual::surface(
                super::super::visual::SurfaceLevel::Recessed,
                cx,
            ))
            .p(px(2.))
            .shadow(vec![box_shadow(
                0.,
                5.,
                14.,
                -10.,
                cx.theme().background.opacity(0.9),
            )])
            .child(
                ToggleGroup::new("prompt-mode-selector")
                    .segmented()
                    .with_size(gpui_kit::component::Size::Small)
                    .child(
                        Toggle::new("prompt-mode-chat")
                            .label("Chat")
                            .w(px(76.))
                            .h(px(28.))
                            .checked(mode == ConversationMode::Chat),
                    )
                    .child(
                        Toggle::new("prompt-mode-work")
                            .label("Work")
                            .w(px(76.))
                            .h(px(28.))
                            .checked(mode == ConversationMode::Agent)
                            .disabled(!agent_available),
                    )
                    .on_click(move |checks, window, cx| {
                        let selects_work = super::super::select_second_segment(
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

    fn workspace_button(&self, view: Entity<Self>, cx: &Context<'_, Self>) -> AnyElement {
        let label = self
            .workspace_root
            .as_ref()
            .and_then(|path| path.file_name())
            .and_then(|name| name.to_str())
            .map_or_else(
                || "Choose workspace".to_owned(),
                |name| format!("Workspace · {name}"),
            );
        let button = Button::new("prompt-workspace")
            .ghost()
            .compact()
            .h(px(30.))
            .px(px(8.))
            .rounded(px(8.))
            .border_1()
            .border_color(cx.theme().foreground.opacity(0.07))
            .bg(cx.theme().accent.opacity(0.34))
            .label(label)
            .tooltip(if self.agent_capability.commands() {
                "Choose the workspace this agent can access"
            } else {
                "Choose a workspace. Sandboxed commands are unavailable; file tools still work"
            })
            .on_click(move |_, window, cx| {
                view.update(cx, |composer, cx| composer.choose_workspace(window, cx));
            });
        button
            .when(self.workspace_root.is_none(), |button| {
                button.text_color(cx.theme().warning)
            })
            .into_any_element()
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

    fn attachment_button(cx: &Context<'_, Self>) -> Button {
        let view = cx.entity();

        Button::new("prompt-add-image")
            .ghost()
            .accessibility_id("prompt-add-image")
            .tooltip("Add photos")
            .size(px(30.))
            .p_0()
            .rounded_full()
            .border_1()
            .border_color(cx.theme().primary.opacity(0.35))
            .bg(cx.theme().accent.opacity(0.48))
            .text_color(cx.theme().primary)
            .icon(IconName::Plus)
            .on_click(move |_, window, cx| {
                view.update(cx, |composer, cx| {
                    composer.choose_attachments(window, cx);
                });
            })
    }

    fn model_selector(&self, view: Entity<Self>, cx: &App) -> AnyElement {
        let selected_model = self.model.clone();
        let selected_effort = self.effort.clone();
        let models = self.models.clone();
        let selected_model_id = selected_model.as_ref().map(|model| model.id.clone());
        let efforts = selected_model
            .as_ref()
            .map_or_else(Vec::new, |model| model.supported_efforts.clone());
        let model_label: SharedString = selected_model.as_ref().map_or_else(
            || "Choose model".into(),
            |model| model.display_name.clone().into(),
        );
        let effort_label: SharedString = selected_effort.as_ref().map_or_else(
            || "Effort".into(),
            |effort| effort.label().to_owned().into(),
        );
        let selected_provider = selected_model.as_ref().map_or_else(
            || provider_icon(None),
            |model| provider_icon(Some(&model.provider)),
        );
        let trigger = Button::new("prompt-model")
            .ghost()
            .accessibility_id("prompt-model-and-effort-selector")
            .debug_selector(|| "prompt-model-selector".into())
            .h(px(32.))
            .max_w(px(260.))
            .px(px(10.))
            .gap(px(7.))
            .rounded(px(9.))
            .border_1()
            .border_color(cx.theme().foreground.opacity(0.08))
            .bg(super::super::visual::surface(
                super::super::visual::SurfaceLevel::Raised,
                cx,
            ))
            .child(selected_provider)
            .child(
                div()
                    .min_w_0()
                    .text_size(px(12.))
                    .font_medium()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .child(model_label),
            )
            .child(
                div()
                    .flex_none()
                    .text_size(px(11.))
                    .text_color(cx.theme().muted_foreground)
                    .child(effort_label),
            )
            .child(
                Icon::new(IconName::ChevronDown)
                    .xsmall()
                    .text_color(cx.theme().muted_foreground),
            );

        Popover::new("prompt-model-picker")
            .anchor(Anchor::BottomRight)
            .trigger(trigger)
            .appearance(false)
            .content(move |_, window, popover_cx| {
                Self::model_picker_surface(
                    &models,
                    selected_model_id.as_ref(),
                    &efforts,
                    selected_effort.as_ref(),
                    &view,
                    window,
                    popover_cx,
                )
            })
            .into_any_element()
    }

    fn model_picker_surface(
        models: &[ModelDescriptor],
        selected_model_id: Option<&ModelId>,
        efforts: &[EffortLevel],
        selected_effort: Option<&EffortLevel>,
        view: &Entity<Self>,
        window: &Window,
        cx: &Context<'_, PopoverState>,
    ) -> AnyElement {
        let popover = cx.entity();
        let model_rows = models
            .iter()
            .map(|model| Self::model_picker_model_row(model, selected_model_id, view, window, cx));
        let effort_rows = efforts.iter().map(|effort| {
            Self::model_picker_effort_row(effort, selected_effort, view, &popover, cx)
        });

        h_flex()
            .debug_selector(|| "prompt-model-picker-surface".into())
            .w(px(424.))
            .max_h(px(420.))
            .items_stretch()
            .overflow_hidden()
            .rounded(px(14.))
            .border_1()
            .border_color(cx.theme().foreground.opacity(0.09))
            .bg(super::super::visual::surface(
                super::super::visual::SurfaceLevel::Floating,
                cx,
            ))
            .shadow(super::super::visual::floating_shadow(cx))
            .child(
                v_flex()
                    .w(px(256.))
                    .min_h(px(150.))
                    .p(px(8.))
                    .gap(px(3.))
                    .child(picker_heading("Models", cx))
                    .children(model_rows),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_h(px(150.))
                    .border_l_1()
                    .border_color(cx.theme().foreground.opacity(0.07))
                    .p(px(8.))
                    .gap(px(3.))
                    .child(picker_heading("Effort", cx))
                    .children(effort_rows),
            )
            .into_any_element()
    }

    fn model_picker_model_row(
        model: &ModelDescriptor,
        selected_model_id: Option<&ModelId>,
        view: &Entity<Self>,
        window: &Window,
        cx: &App,
    ) -> Button {
        let selected = selected_model_id == Some(&model.id);
        let model_for_click = model.clone();

        Button::new(SharedString::from(format!("model-{}", model.id.0)))
            .ghost()
            .w_full()
            .h(px(44.))
            .px(px(9.))
            .rounded(px(9.))
            .border_1()
            .border_color(if selected {
                cx.theme().primary.opacity(0.28)
            } else {
                cx.theme().foreground.opacity(0.)
            })
            .bg(if selected {
                cx.theme().accent.opacity(0.72)
            } else {
                cx.theme().accent.opacity(0.)
            })
            .child(
                h_flex()
                    .w_full()
                    .items_center()
                    .gap(px(9.))
                    .child(provider_icon(Some(&model.provider)))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap(px(2.))
                            .child(
                                div()
                                    .w_full()
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .text_size(px(12.))
                                    .font_medium()
                                    .child(model.display_name.clone()),
                            )
                            .child(
                                div()
                                    .text_size(px(10.))
                                    .text_color(cx.theme().muted_foreground)
                                    .child(provider_menu_label(&model.provider)),
                            ),
                    )
                    .when(selected, |this| {
                        this.child(div().size(px(5.)).rounded_full().bg(cx.theme().primary))
                    }),
            )
            .on_click(window.listener_for(view, move |composer, _, _, cx| {
                composer.select_model(model_for_click.clone(), cx);
            }))
    }

    fn model_picker_effort_row(
        effort: &EffortLevel,
        selected_effort: Option<&EffortLevel>,
        view: &Entity<Self>,
        popover: &Entity<PopoverState>,
        cx: &App,
    ) -> Button {
        let selected = selected_effort == Some(effort);
        let effort_for_click = effort.clone();
        let select_view = view.clone();
        let dismiss = popover.clone();

        Button::new(SharedString::from(format!(
            "effort-{}",
            effort.wire_value()
        )))
        .ghost()
        .w_full()
        .h(px(44.))
        .px(px(9.))
        .rounded(px(9.))
        .border_1()
        .border_color(if selected {
            cx.theme().primary.opacity(0.28)
        } else {
            cx.theme().foreground.opacity(0.)
        })
        .bg(if selected {
            cx.theme().accent.opacity(0.72)
        } else {
            cx.theme().accent.opacity(0.)
        })
        .child(
            h_flex()
                .w_full()
                .items_center()
                .justify_between()
                .child(
                    v_flex()
                        .items_start()
                        .gap(px(2.))
                        .child(
                            div()
                                .text_size(px(12.))
                                .font_medium()
                                .child(effort.label().to_owned()),
                        )
                        .child(
                            div()
                                .text_size(px(10.))
                                .text_color(cx.theme().muted_foreground)
                                .child(effort_description(effort)),
                        ),
                )
                .when(selected, |this| {
                    this.child(div().size(px(5.)).rounded_full().bg(cx.theme().primary))
                }),
        )
        .on_click(move |_, window, cx| {
            select_view.update(cx, |composer, cx| {
                composer.select_effort(effort_for_click.clone(), cx);
            });
            dismiss.update(cx, |state, cx| state.dismiss(window, cx));
        })
    }

    fn code_preview(&self, cx: &Context<'_, Self>) -> impl IntoElement {
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

    fn footer(
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

impl Render for PromptComposer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let _ = &self.subscriptions;
        self.synchronize_input_mode(window, cx);

        let focused = self.input.read(cx).focus_handle(cx).is_focused(window);
        let ready = self.is_ready(cx) && !self.generating;
        let submit_view = cx.entity();
        let generating = self.generating;
        let is_work = self.mode == ConversationMode::Agent;
        let can_add_attachment = !is_work && self.attachments.len() < MAX_ATTACHMENTS;
        let highlight = linear_gradient(
            90.,
            linear_color_stop(cx.theme().primary.opacity(0.), 0.),
            linear_color_stop(
                cx.theme()
                    .primary
                    .opacity(if focused { 0.92 } else { 0.48 }),
                1.,
            ),
        );

        v_flex()
            .w_full()
            .max_w(px(800.))
            .mx_auto()
            .items_center()
            .gap(px(14.))
            .child(self.mode_selector(cx.entity(), cx))
            .child(
                v_flex()
                    .id("prompt-composer-surface")
                    .debug_selector(|| "prompt-composer-surface".into())
                    .relative()
                    .w_full()
                    .p(px(2.))
                    .rounded(if is_work { px(22.) } else { px(19.) })
                    .border_1()
                    .border_color(if focused {
                        cx.theme().ring.opacity(0.78)
                    } else {
                        cx.theme().foreground.opacity(0.08)
                    })
                    .bg(super::super::visual::surface(
                        super::super::visual::SurfaceLevel::Floating,
                        cx,
                    ))
                    .shadow(super::super::visual::floating_shadow(cx))
                    .child(
                        div()
                            .absolute()
                            .top(px(0.))
                            .left(px(24.))
                            .right(px(24.))
                            .h(px(1.))
                            .bg(highlight),
                    )
                    .child(
                        v_flex()
                            .w_full()
                            .min_h(if is_work { px(124.) } else { px(56.) })
                            .p(if is_work { px(16.) } else { px(8.) })
                            .gap(if is_work { px(12.) } else { px(0.) })
                            .justify_between()
                            .rounded(if is_work { px(19.) } else { px(16.) })
                            .border_1()
                            .border_color(cx.theme().foreground.opacity(0.045))
                            .bg(super::super::visual::surface(
                                super::super::visual::SurfaceLevel::Raised,
                                cx,
                            ))
                            .shadow(super::super::visual::raised_shadow(cx))
                            .when(is_work, |this| {
                                this.child(self.render_input_content(cx)).child(self.footer(
                                    submit_view.clone(),
                                    generating,
                                    ready,
                                    false,
                                    cx,
                                ))
                            })
                            .when(!is_work, |this| {
                                this.child(self.render_chat_content(
                                    submit_view,
                                    generating,
                                    ready,
                                    can_add_attachment,
                                    cx,
                                ))
                            }),
                    ),
            )
    }
}

fn provider_menu_label(provider: &ProviderId) -> String {
    let id = provider.0.to_ascii_lowercase();
    if id.starts_with("openai") {
        "OpenAI".to_owned()
    } else if id.starts_with("anthropic") || id.starts_with("claude") {
        "Anthropic".to_owned()
    } else if id.starts_with("google") || id.starts_with("gemini") {
        "Gemini".to_owned()
    } else {
        provider.0.clone()
    }
}

fn picker_heading(label: &'static str, cx: &App) -> AnyElement {
    div()
        .h(px(28.))
        .px(px(9.))
        .flex()
        .items_center()
        .text_size(px(10.))
        .font_semibold()
        .text_color(cx.theme().muted_foreground)
        .child(label)
        .into_any_element()
}

const fn effort_description(effort: &EffortLevel) -> &'static str {
    match effort {
        EffortLevel::None | EffortLevel::Minimal => "Fastest",
        EffortLevel::Low => "Quick",
        EffortLevel::Medium => "Balanced",
        EffortLevel::High => "Deeper",
        EffortLevel::XHigh | EffortLevel::Max => "Deepest",
        EffortLevel::Custom { .. } => "Custom",
    }
}
