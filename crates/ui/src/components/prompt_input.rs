use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
    time::Duration,
};

use gpui::{
    App, AppContext as _, Context, Entity, EventEmitter, Focusable as _, InteractiveElement as _,
    IntoElement, ObjectFit, ParentElement as _, PathPromptOptions, Render, SharedString,
    Styled as _, StyledImage as _, Subscription, Task, Window, div, img, linear_color_stop,
    linear_gradient, prelude::FluentBuilder as _, px, rems,
};
use gpui_component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _, WindowExt,
    button::{Button, ButtonVariants},
    clipboard::Clipboard,
    h_flex,
    input::{InputEvent, Textarea, TextareaState},
    menu::{DropdownMenu as _, PopupMenuItem},
    notification::{Notification, NotificationType},
    text::{TextView, TextViewState, TextViewStyle},
    v_flex,
};

use magenta_core::EffortLevel;

use crate::{MagentaError, components::code_fence, notification_for_error};

const MAX_ATTACHMENTS: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChatModel {
    Sonnet,
    Gpt,
    GeminiPro,
}

impl ChatModel {
    const ALL: [Self; 3] = [Self::Sonnet, Self::Gpt, Self::GeminiPro];

    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Sonnet => "Sonnet",
            Self::Gpt => "GPT",
            Self::GeminiPro => "Gemini Pro",
        }
    }

    pub(crate) const fn id(self) -> &'static str {
        match self {
            Self::Sonnet => "sonnet",
            Self::Gpt => "gpt",
            Self::GeminiPro => "gemini-pro",
        }
    }

    pub(crate) fn from_id(id: &str) -> Self {
        match id {
            "gpt" => Self::Gpt,
            "gemini-pro" => Self::GeminiPro,
            _ => Self::Sonnet,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ReferenceImage {
    id: u64,
    path: PathBuf,
    name: SharedString,
}

impl ReferenceImage {
    fn new(path: PathBuf) -> Self {
        let mut hasher = DefaultHasher::new();
        path.hash(&mut hasher);
        let id = hasher.finish();
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("Reference image")
            .to_owned()
            .into();

        Self { id, path, name }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PromptRequest {
    pub prompt: SharedString,
    pub model: ChatModel,
    pub effort: EffortLevel,
    pub attachments: Vec<PathBuf>,
}

#[derive(Clone, Debug)]
pub enum PromptComposerEvent {
    Submit(PromptRequest),
    Cancel,
}

pub struct PromptComposer {
    input: Entity<TextareaState>,
    preview: Entity<TextViewState>,
    preview_source: String,
    preview_line_count: usize,
    model: Option<ChatModel>,
    effort: Option<EffortLevel>,
    generating: bool,
    attachments: Vec<ReferenceImage>,
    attachment_task: Option<Task<()>>,
    preview_task: Option<Task<()>>,
    preview_generation: u64,
    subscriptions: Vec<Subscription>,
}

impl PromptComposer {
    pub fn new(window: &mut Window, cx: &mut Context<'_, Self>) -> Self {
        let input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("Message Magenta")
                .auto_grow(2, 5)
                .submit_on_enter(true)
        });
        let preview = cx.new(|cx| TextViewState::markdown("", cx));

        let subscriptions = vec![cx.subscribe_in(
            &input,
            window,
            |composer, _, event: &InputEvent, window, cx| match event {
                InputEvent::Change => composer.schedule_code_preview(window, cx),
                InputEvent::Focus | InputEvent::Blur => cx.notify(),
                InputEvent::PressEnter { shift: false, .. } => composer.submit(cx),
                InputEvent::PressEnter { shift: true, .. } => {
                    composer.handle_shift_enter(window, cx);
                }
            },
        )];

        Self {
            input,
            preview,
            preview_source: String::new(),
            preview_line_count: 0,
            model: Some(ChatModel::Sonnet),
            effort: Some(EffortLevel::Medium),
            generating: false,
            attachments: Vec::new(),
            attachment_task: None,
            preview_task: None,
            preview_generation: 0,
            subscriptions,
        }
    }

    pub fn focus(&self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.input.focus_handle(cx).focus(window, cx);
    }

    pub fn set_configuration(
        &mut self,
        model: ChatModel,
        effort: EffortLevel,
        cx: &mut Context<'_, Self>,
    ) {
        self.model = Some(model);
        self.effort = Some(effort);
        cx.notify();
    }

    pub fn set_generating(&mut self, generating: bool, cx: &mut Context<'_, Self>) {
        if self.generating != generating {
            self.generating = generating;
            cx.notify();
        }
    }

    pub fn clear_after_submit(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.input.update(cx, |input, cx| {
            input.set_value("", window, cx);
        });
        self.preview_generation = self.preview_generation.wrapping_add(1);
        self.preview_task.take();
        self.clear_code_preview(cx);
        self.attachments.clear();
        cx.notify();
    }

    fn schedule_code_preview(&mut self, window: &Window, cx: &mut Context<'_, Self>) {
        self.preview_generation = self.preview_generation.wrapping_add(1);
        let generation = self.preview_generation;
        self.preview_task.take();

        let source = self.input.read(cx).value().to_string();
        if !source.contains("```") {
            self.clear_code_preview(cx);
            cx.notify();
            return;
        }

        self.preview_task = Some(cx.spawn_in(window, async move |composer, window| {
            window
                .background_executor()
                .timer(Duration::from_millis(60))
                .await;

            let blocks = window
                .background_executor()
                .spawn(async move { code_fence::fenced_blocks(&source) })
                .await;

            _ = composer.update_in(window, |composer, _, cx| {
                if composer.preview_generation != generation {
                    return;
                }

                composer.preview_task = None;
                composer.apply_code_preview(&blocks, cx);
            });
        }));
        cx.notify();
    }

    fn apply_code_preview(
        &mut self,
        blocks: &[code_fence::FencedCodeBlock],
        cx: &mut Context<'_, Self>,
    ) {
        let source = code_fence::preview_markdown(blocks);
        if self.preview_source == source {
            return;
        }

        self.preview_line_count = source.lines().count();
        self.preview_source.clone_from(&source);
        self.preview.update(cx, |preview, cx| {
            preview.set_text(&source, cx);
        });
        cx.notify();
    }

    fn clear_code_preview(&mut self, cx: &mut Context<'_, Self>) {
        if self.preview_source.is_empty() {
            return;
        }

        self.preview_source.clear();
        self.preview_line_count = 0;
        self.preview.update(cx, |preview, cx| {
            preview.set_text("", cx);
        });
    }

    fn handle_shift_enter(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let (value, cursor) = {
            let input = self.input.read(cx);
            (input.value(), input.cursor())
        };
        let Some(auto_close) = code_fence::opening_fence_after_newline(&value, cursor) else {
            self.schedule_code_preview(window, cx);
            return;
        };

        self.input.update(cx, |input, cx| {
            input.insert(auto_close.insertion, window, cx);
            input.set_selected_range(cursor..cursor, cx);
        });
        self.schedule_code_preview(window, cx);
    }

    fn has_content(&self, cx: &App) -> bool {
        !self.input.read(cx).value().trim().is_empty() || !self.attachments.is_empty()
    }

    fn is_ready(&self, cx: &App) -> bool {
        self.has_content(cx) && self.model.is_some() && self.effort.is_some()
    }

    fn select_model(&mut self, model: ChatModel, cx: &mut Context<'_, Self>) {
        if self.model != Some(model) {
            self.model = Some(model);
            cx.notify();
        }
    }

    fn select_effort(&mut self, effort: EffortLevel, cx: &mut Context<'_, Self>) {
        if self.effort != Some(effort) {
            self.effort = Some(effort);
            cx.notify();
        }
    }

    fn choose_attachments(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.attachments.len() >= MAX_ATTACHMENTS {
            window.push_notification(
                Notification::new()
                    .title("Four images already attached")
                    .message("Remove an image before adding another attachment.")
                    .with_type(NotificationType::Warning),
                cx,
            );
            return;
        }

        let picker = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Add images".into()),
        });

        self.attachment_task = Some(cx.spawn_in(window, async move |composer, window| {
            let selection = match picker.await {
                Ok(Ok(paths)) => paths,
                Ok(Err(source)) => {
                    _ = composer.update_in(window, |composer, window, cx| {
                        composer.attachment_task = None;
                        let error = MagentaError::AttachmentPicker { source };
                        window.push_notification(notification_for_error(&error), cx);
                    });
                    return;
                }
                Err(_) => return,
            };

            let Some(paths) = selection else {
                _ = composer.update_in(window, |composer, _, _| {
                    composer.attachment_task = None;
                });
                return;
            };

            let paths = window
                .background_executor()
                .spawn(async move {
                    paths
                        .into_iter()
                        .map(|path| {
                            let readable = std::fs::File::open(&path).is_ok();
                            (path, readable)
                        })
                        .collect::<Vec<_>>()
                })
                .await;

            _ = composer.update_in(window, |composer, window, cx| {
                composer.add_attachments(paths, window, cx);
                composer.attachment_task = None;
            });
        }));
    }

    fn add_attachments(
        &mut self,
        paths: Vec<(PathBuf, bool)>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let mut unsupported = 0;
        let mut unreadable = 0;
        let mut duplicates = 0;
        let mut overflow = 0;

        for (path, readable) in paths {
            if !is_supported_image(&path) {
                unsupported += 1;
                continue;
            }

            if !readable {
                unreadable += 1;
                continue;
            }

            if self
                .attachments
                .iter()
                .any(|attachment| attachment.path == path)
            {
                duplicates += 1;
                continue;
            }

            if self.attachments.len() >= MAX_ATTACHMENTS {
                overflow += 1;
                continue;
            }

            self.attachments.push(ReferenceImage::new(path));
        }

        let skipped = unsupported + unreadable + duplicates + overflow;
        if skipped > 0 {
            let message = format!(
                "Skipped {skipped} file(s): {unsupported} unsupported, {unreadable} unreadable, {duplicates} duplicate, {overflow} over the four-image limit."
            );
            window.push_notification(
                Notification::new()
                    .title("Some images were not added")
                    .message(message)
                    .with_type(NotificationType::Warning),
                cx,
            );
        }

        cx.notify();
    }

    fn remove_attachment(&mut self, path: &Path, cx: &mut Context<'_, Self>) {
        let before = self.attachments.len();
        self.attachments
            .retain(|attachment| attachment.path != *path);
        if self.attachments.len() != before {
            cx.notify();
        }
    }

    fn request(&self, cx: &App) -> Option<PromptRequest> {
        if !self.is_ready(cx) {
            return None;
        }

        Some(PromptRequest {
            prompt: self.input.read(cx).value().trim().to_owned().into(),
            model: self.model?,
            effort: self.effort?,
            attachments: self
                .attachments
                .iter()
                .map(|attachment| attachment.path.clone())
                .collect(),
        })
    }

    fn submit(&self, cx: &mut Context<'_, Self>) {
        if self.generating {
            return;
        }

        let Some(request) = self.request(cx) else {
            return;
        };

        cx.emit(PromptComposerEvent::Submit(request));
    }

    fn cancel(&self, cx: &mut Context<'_, Self>) {
        if self.generating {
            cx.emit(PromptComposerEvent::Cancel);
        }
    }

    fn attachment_strip(&self, cx: &Context<'_, Self>) -> impl IntoElement {
        let view = cx.entity();
        let can_add = self.attachments.len() < MAX_ATTACHMENTS;

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
            .when(can_add, |this| {
                let add_view = view.clone();
                this.child(
                    Button::new("prompt-add-image")
                        .ghost()
                        .accessibility_id("prompt-add-image")
                        .tooltip("Add images")
                        .size(px(34.))
                        .p_0()
                        .rounded(px(7.))
                        .border_1()
                        .border_color(cx.theme().border.opacity(0.88))
                        .bg(cx.theme().muted.opacity(0.72))
                        .icon(IconName::GalleryVerticalEnd)
                        .on_click(move |_, window, cx| {
                            add_view.update(cx, |composer, cx| {
                                composer.choose_attachments(window, cx);
                            });
                        }),
                )
            })
    }

    fn model_menu(&self, cx: &Context<'_, Self>) -> impl IntoElement {
        let selected_model = self.model;
        let selected_effort = self.effort;
        let view = cx.entity();
        let trigger_label: SharedString = match (selected_model, selected_effort) {
            (None, None) => "Choose model".into(),
            (Some(model), None) => format!("{}  ·  Choose effort", model.label()).into(),
            (None, Some(effort)) => format!("Choose model  ·  {}", effort.label()).into(),
            (Some(model), Some(effort)) => {
                format!("{}  ·  {}", model.label(), effort.label()).into()
            }
        };

        option_button("prompt-model", trigger_label, IconName::Bot)
            .accessibility_id("prompt-model-and-effort-selector")
            .dropdown_menu(move |menu, window, cx| {
                let menu = ChatModel::ALL.into_iter().fold(
                    menu.min_w(px(210.)).label("Models"),
                    |menu, model| {
                        let select_view = view.clone();
                        menu.item(
                            PopupMenuItem::new(model.label())
                                .checked(selected_model == Some(model))
                                .on_click(window.listener_for(
                                    &select_view,
                                    move |composer, _, _, cx| {
                                        composer.select_model(model, cx);
                                    },
                                )),
                        )
                    },
                );

                let effort_view = view.clone();
                let effort_label = selected_effort.map_or_else(
                    || "Effort".to_owned(),
                    |effort| format!("Effort  ·  {}", effort.label()),
                );

                menu.separator()
                    .submenu(effort_label, window, cx, move |menu, window, _| {
                        EffortLevel::ALL.into_iter().fold(
                            menu.min_w(px(150.)).label("Effort level"),
                            |menu, effort| {
                                let select_view = effort_view.clone();
                                menu.item(
                                    PopupMenuItem::new(effort.label())
                                        .checked(selected_effort == Some(effort))
                                        .on_click(window.listener_for(
                                            &select_view,
                                            move |composer, _, _, cx| {
                                                composer.select_effort(effort, cx);
                                            },
                                        )),
                                )
                            },
                        )
                    })
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
}

impl EventEmitter<PromptComposerEvent> for PromptComposer {}

impl Render for PromptComposer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let _ = &self.subscriptions;
        let focused = self.input.read(cx).focus_handle(cx).is_focused(window);
        let ready = self.is_ready(cx) && !self.generating;
        let submit_view = cx.entity();
        let generating = self.generating;

        v_flex()
            .w_full()
            .min_h(px(112.))
            .p(px(10.))
            .gap(px(8.))
            .justify_between()
            .rounded(px(9.))
            .border_1()
            .border_color(if focused {
                cx.theme().ring.opacity(0.74)
            } else {
                cx.theme().border.opacity(0.76)
            })
            .bg(linear_gradient(
                145.,
                linear_color_stop(cx.theme().button.opacity(0.98), 0.),
                linear_color_stop(cx.theme().secondary.opacity(0.84), 1.),
            ))
            .shadow_lg()
            .child(
                v_flex()
                    .flex_1()
                    .min_h_0()
                    .gap(px(7.))
                    .child(self.attachment_strip(cx))
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
                            .min_h(px(38.))
                            .p_0()
                            .text_size(px(12.))
                            .line_height(px(18.)),
                    ),
            )
            .child(
                h_flex()
                    .w_full()
                    .items_center()
                    .justify_between()
                    .gap(px(10.))
                    .child(h_flex().min_w_0().gap(px(5.)).child(self.model_menu(cx)))
                    .child(
                        Button::new("prompt-submit")
                            .when(generating, ButtonVariants::secondary)
                            .when(!generating, ButtonVariants::primary)
                            .disabled(!ready && !generating)
                            .accessibility_id(if generating {
                                "prompt-cancel"
                            } else {
                                "prompt-submit"
                            })
                            .tooltip(if generating {
                                "Stop generating"
                            } else if ready {
                                "Send message"
                            } else {
                                "Add a message before sending"
                            })
                            .size(px(36.))
                            .p_0()
                            .rounded_full()
                            .icon(if generating {
                                IconName::CircleX
                            } else {
                                IconName::ChevronUp
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
    }
}

fn option_button(id: &'static str, label: impl Into<SharedString>, icon: IconName) -> Button {
    Button::new(id)
        .compact()
        .dropdown_caret(true)
        .h(px(28.))
        .px(px(8.))
        .rounded(px(6.))
        .icon(icon)
        .label(label)
}

fn is_supported_image(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "png" | "jpg" | "jpeg" | "webp"
            )
        })
}

#[cfg(test)]
mod tests {
    use gpui::{TestAppContext, size};

    use super::*;

    #[test]
    fn supported_image_extensions_are_case_insensitive() {
        assert!(is_supported_image(std::path::Path::new("reference.PNG")));
        assert!(is_supported_image(std::path::Path::new("reference.jpeg")));
        assert!(is_supported_image(std::path::Path::new("reference.WebP")));
        assert!(!is_supported_image(std::path::Path::new("reference.gif")));
        assert!(!is_supported_image(std::path::Path::new("reference")));
    }

    #[gpui::test]
    fn composer_requires_content_model_and_effort(cx: &mut TestAppContext) {
        cx.update(gpui_component::init);
        let window = cx.open_window(size(px(720.), px(420.)), PromptComposer::new);

        window
            .update(cx, |composer, window, cx| {
                assert!(!composer.is_ready(cx));

                composer.input.update(cx, |input, cx| {
                    input.set_value("   ", window, cx);
                });
                composer.select_model(ChatModel::Sonnet, cx);
                composer.select_effort(EffortLevel::Medium, cx);
                assert!(!composer.is_ready(cx));

                composer.input.update(cx, |input, cx| {
                    input.set_value("A luminous glass sphere", window, cx);
                });
                assert!(composer.is_ready(cx));
            })
            .expect("the composer test window should remain open");
    }

    #[gpui::test]
    fn request_trims_prompt_and_preserves_configuration(cx: &mut TestAppContext) {
        cx.update(gpui_component::init);
        let window = cx.open_window(size(px(720.), px(420.)), PromptComposer::new);

        window
            .update(cx, |composer, window, cx| {
                composer.input.update(cx, |input, cx| {
                    input.set_value("  A quiet cyan horizon  ", window, cx);
                });
                composer.select_model(ChatModel::GeminiPro, cx);
                composer.select_effort(EffortLevel::High, cx);

                let request = composer.request(cx).expect("the request should be ready");
                assert_eq!(request.prompt.as_ref(), "A quiet cyan horizon");
                assert_eq!(request.model, ChatModel::GeminiPro);
                assert_eq!(request.effort, EffortLevel::High);
                assert!(request.attachments.is_empty());
            })
            .expect("the composer test window should remain open");
    }

    #[gpui::test]
    fn shift_enter_pairs_a_fence_and_keeps_the_caret_in_the_code_body(cx: &mut TestAppContext) {
        cx.update(gpui_component::init);
        let window = cx.open_window(size(px(720.), px(420.)), PromptComposer::new);

        window
            .update(cx, |composer, window, cx| {
                let opening = "```rust\n";
                composer.input.update(cx, |input, cx| {
                    input.set_value(opening, window, cx);
                    input.set_selected_range(opening.len()..opening.len(), cx);
                });

                composer.handle_shift_enter(window, cx);

                let input = composer.input.read(cx);
                assert_eq!(input.value().as_ref(), "```rust\n\n```");
                assert_eq!(input.cursor(), opening.len());
            })
            .expect("the composer test window should remain open");
    }
}
