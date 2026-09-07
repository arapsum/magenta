use gpui::{
    AnyElement, App, Context, Entity, Focusable as _, InteractiveElement as _, IntoElement,
    ObjectFit, ParentElement as _, Render, SharedString, Styled as _, StyledImage as _, Window,
    div, img, prelude::FluentBuilder as _, px, rems,
};
use gpui_component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _,
    button::{Button, ButtonVariants},
    clipboard::Clipboard,
    h_flex,
    input::Textarea,
    menu::{DropdownMenu as _, PopupMenuItem},
    text::{TextView, TextViewStyle},
    v_flex,
};
use magenta_core::ConversationMode;

use crate::components::provider_icon;

use super::{MAX_ATTACHMENTS, PromptComposer};

impl PromptComposer {
    fn mode_selector(&self, view: Entity<Self>) -> AnyElement {
        let mode = self.mode.clone();
        let agent_available = self.agent_available;
        let label = match mode {
            ConversationMode::Chat => "Chat",
            ConversationMode::Agent => "Agent",
        };

        option_button("prompt-mode", label, IconName::Bot)
            .accessibility_id("prompt-mode-selector")
            .dropdown_menu(move |menu, window, _cx| {
                let chat_view = view.clone();
                let agent_view = view.clone();
                menu.min_w(px(170.))
                    .label("Run mode")
                    .item(
                        PopupMenuItem::new("Chat")
                            .checked(mode == ConversationMode::Chat)
                            .on_click(window.listener_for(&chat_view, |composer, _, _, cx| {
                                composer.select_mode(ConversationMode::Chat, cx);
                            })),
                    )
                    .item(
                        PopupMenuItem::new("Agent")
                            .checked(mode == ConversationMode::Agent)
                            .disabled(!agent_available)
                            .on_click(window.listener_for(&agent_view, |composer, _, _, cx| {
                                composer.select_mode(ConversationMode::Agent, cx);
                            })),
                    )
            })
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
            .compact()
            .h(px(28.))
            .px(px(8.))
            .rounded(px(6.))
            .label(label)
            .tooltip("Choose the workspace this agent can access")
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
            .border_color(cx.theme().border)
            .bg(cx.theme().background)
            .icon(IconName::Plus)
            .on_click(move |_, window, cx| {
                view.update(cx, |composer, cx| {
                    composer.choose_attachments(window, cx);
                });
            })
    }

    fn model_selector(&self, view: Entity<Self>, _cx: &App) -> AnyElement {
        let selected_model = self.model.clone();
        let selected_effort = self.effort.clone();
        let models = self.models.clone();
        let selected_model_id = selected_model.as_ref().map(|model| model.id.clone());
        let efforts = selected_model
            .as_ref()
            .map_or_else(Vec::new, |model| model.supported_efforts.clone());
        let trigger_label: SharedString = match (selected_model.as_ref(), selected_effort.as_ref())
        {
            (None, None) => "Choose model".into(),
            (Some(model), None) => format!("{}  ·  Choose effort", model.display_name).into(),
            (None, Some(effort)) => format!("Choose model  ·  {}", effort.label()).into(),
            (Some(model), Some(effort)) => {
                format!("{}  ·  {}", model.display_name, effort.label()).into()
            }
        };
        let selected_provider = selected_model.as_ref().map_or_else(
            || provider_icon(None),
            |model| provider_icon(Some(&model.provider)),
        );

        option_button("prompt-model", trigger_label, selected_provider)
            .accessibility_id("prompt-model-and-effort-selector")
            .dropdown_menu(move |menu, window, cx| {
                let menu = models.clone().into_iter().fold(
                    menu.min_w(px(230.)).label("Models"),
                    |menu, model| {
                        let select_view = view.clone();
                        let model_for_click = model.clone();
                        menu.item(
                            PopupMenuItem::new(model.display_name.clone())
                                .icon(provider_icon(Some(&model.provider)))
                                .checked(selected_model_id.as_ref() == Some(&model.id))
                                .on_click(window.listener_for(
                                    &select_view,
                                    move |composer, _, _, cx| {
                                        composer.select_model(model_for_click.clone(), cx);
                                    },
                                )),
                        )
                    },
                );

                let effort_view = view.clone();
                let effort_label = selected_effort.as_ref().map_or_else(
                    || "Effort".to_owned(),
                    |effort| format!("Effort  ·  {}", effort.label()),
                );

                menu.separator().submenu(effort_label, window, cx, {
                    let efforts = efforts.clone();
                    let selected_effort = selected_effort.clone();
                    move |menu, window, _| {
                        efforts.clone().into_iter().fold(
                            menu.min_w(px(150.)).label("Effort level"),
                            |menu, effort| {
                                let select_view = effort_view.clone();
                                let effort_for_click = effort.clone();
                                menu.item(
                                    PopupMenuItem::new(effort.label())
                                        .checked(selected_effort.as_ref() == Some(&effort))
                                        .on_click(window.listener_for(
                                            &select_view,
                                            move |composer, _, _, cx| {
                                                composer
                                                    .select_effort(effort_for_click.clone(), cx);
                                            },
                                        )),
                                )
                            },
                        )
                    }
                })
            })
            .into_any_element()
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
                    .gap(px(4.))
                    .when(can_add_attachment, |this| {
                        this.child(Self::attachment_button(cx))
                    })
                    .child(self.mode_selector(submit_view.clone()))
                    .when(self.mode == ConversationMode::Agent, |this| {
                        this.child(self.workspace_button(submit_view.clone(), cx))
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
                            .size(px(30.))
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
        let focused = self.input.read(cx).focus_handle(cx).is_focused(window);
        let ready = self.is_ready(cx) && !self.generating;
        let submit_view = cx.entity();
        let generating = self.generating;
        let can_add_attachment = self.attachments.len() < MAX_ATTACHMENTS;

        v_flex()
            .id("prompt-composer-surface")
            .debug_selector(|| "prompt-composer-surface".into())
            .w_full()
            .max_w(px(672.))
            .mx_auto()
            .min_h(px(104.))
            .p(px(14.))
            .gap(px(8.))
            .justify_between()
            .rounded(px(18.))
            .border_1()
            .border_color(if focused {
                cx.theme().ring
            } else {
                cx.theme().border.opacity(0.82)
            })
            .bg(cx.theme().popover)
            .when(focused, gpui::Styled::shadow_md)
            .when(!focused, gpui::Styled::shadow_sm)
            .child(
                v_flex()
                    .flex_1()
                    .min_h_0()
                    .gap(px(7.))
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
                            .text_size(px(14.))
                            .line_height(px(21.)),
                    ),
            )
            .child(self.footer(submit_view, generating, ready, can_add_attachment, cx))
    }
}

fn option_button(
    id: &'static str,
    label: impl Into<SharedString>,
    icon: impl Into<Icon>,
) -> Button {
    Button::new(id)
        .compact()
        .dropdown_caret(true)
        .h(px(28.))
        .px(px(8.))
        .rounded(px(6.))
        .icon(icon)
        .label(label)
}
