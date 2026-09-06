use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
    time::Duration,
};

use gpui::{
    AnyElement, App, AppContext as _, Context, Entity, EventEmitter, Focusable as _,
    InteractiveElement as _, IntoElement, ObjectFit, ParentElement as _, PathPromptOptions, Render,
    SharedString, Styled as _, StyledImage as _, Subscription, Task, Window, div, img,
    prelude::FluentBuilder as _, px, rems,
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

use magenta_core::{EffortLevel, GenerationConfig, ModelDescriptor};

use crate::{MagentaError, components::code_fence, notification_for_error};

const MAX_ATTACHMENTS: usize = 4;
const MAX_ATTACHMENT_BYTES: u64 = 10 * 1024 * 1024;

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
    pub generation: GenerationConfig,
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
    models: Vec<ModelDescriptor>,
    model: Option<ModelDescriptor>,
    effort: Option<EffortLevel>,
    generating: bool,
    storage_ready: bool,
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
                .placeholder("Ask anything")
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
            models: Vec::new(),
            model: None,
            effort: None,
            generating: false,
            storage_ready: true,
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
        configuration: &GenerationConfig,
        cx: &mut Context<'_, Self>,
    ) {
        let model = self
            .models
            .iter()
            .find(|model| {
                model.provider.eq(&configuration.provider) && model.id.eq(&configuration.model)
            })
            .cloned()
            .or_else(|| self.models.first().cloned())
            .unwrap_or_else(|| ModelDescriptor {
                provider: configuration.provider.clone(),
                id: configuration.model.clone(),
                display_name: configuration.model.0.clone(),
                description: None,
                priority: 0,
                default_effort: configuration.effort.clone(),
                supported_efforts: EffortLevel::ALL.to_vec(),
            });
        let uses_requested_model = if model.provider == configuration.provider {
            model.id == configuration.model
        } else {
            false
        };
        self.effort = model
            .supported_efforts
            .contains(&configuration.effort)
            .then_some(configuration.effort.clone())
            .filter(|_| uses_requested_model)
            .or_else(|| Some(model.default_effort.clone()));
        self.model = Some(model);
        cx.notify();
    }

    pub(crate) fn set_models(&mut self, models: Vec<ModelDescriptor>, cx: &mut Context<'_, Self>) {
        let selected = self.model.as_ref().and_then(|selected| {
            models
                .iter()
                .find(|model| model.provider == selected.provider && model.id == selected.id)
                .cloned()
        });
        self.models = models;
        self.model = selected.or_else(|| self.models.first().cloned());
        let current_effort = self.effort.clone();
        self.effort = self.model.as_ref().map(|model| {
            current_effort
                .filter(|effort| model.supported_efforts.contains(effort))
                .unwrap_or_else(|| model.default_effort.clone())
        });
        cx.notify();
    }

    pub fn set_generating(&mut self, generating: bool, cx: &mut Context<'_, Self>) {
        if self.generating != generating {
            self.generating = generating;
            cx.notify();
        }
    }

    pub(crate) fn set_storage_ready(&mut self, ready: bool, cx: &mut Context<'_, Self>) {
        self.storage_ready = ready;
        cx.notify();
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

    pub(crate) fn clear_submitted(
        &mut self,
        submitted: &PromptRequest,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let same_prompt = self.input.read(cx).value().trim() == submitted.prompt.as_ref();
        let same_attachments = self
            .attachments
            .iter()
            .map(|image| &image.path)
            .eq(submitted.attachments.iter());
        if same_prompt && same_attachments {
            self.clear_after_submit(window, cx);
        }
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
        self.storage_ready && self.has_content(cx) && self.model.is_some() && self.effort.is_some()
    }

    fn select_model(&mut self, model: ModelDescriptor, cx: &mut Context<'_, Self>) {
        if self.model.as_ref() != Some(&model) {
            self.effort = Some(model.default_effort.clone());
            self.model = Some(model);
            cx.notify();
        }
    }

    fn select_effort(&mut self, effort: EffortLevel, cx: &mut Context<'_, Self>) {
        if self
            .model
            .as_ref()
            .is_some_and(|model| model.supported_efforts.contains(&effort))
            && self.effort.as_ref() != Some(&effort)
        {
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
                            let metadata = std::fs::metadata(&path).ok();
                            let readable =
                                metadata.as_ref().is_some_and(std::fs::Metadata::is_file)
                                    && std::fs::File::open(&path).is_ok();
                            let byte_size = metadata.map(|metadata| metadata.len());
                            (path, readable, byte_size)
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
        paths: Vec<(PathBuf, bool, Option<u64>)>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let mut unsupported = 0;
        let mut unreadable = 0;
        let mut too_large = 0;
        let mut duplicates = 0;
        let mut overflow = 0;

        for (path, readable, byte_size) in paths {
            if !is_supported_image(&path) {
                unsupported += 1;
                continue;
            }

            if !readable {
                unreadable += 1;
                continue;
            }

            if byte_size.is_none_or(|size| size > MAX_ATTACHMENT_BYTES) {
                too_large += 1;
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

        let skipped = unsupported + unreadable + too_large + duplicates + overflow;
        if skipped > 0 {
            let message = format!(
                concat!(
                    "Skipped {skipped} file(s): {unsupported} unsupported, ",
                    "{unreadable} unreadable, {too_large} over 10 MiB, {duplicates} duplicate, ",
                    "{overflow} over the four-image limit."
                ),
                skipped = skipped,
                unsupported = unsupported,
                unreadable = unreadable,
                too_large = too_large,
                duplicates = duplicates,
                overflow = overflow
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

        let model = self.model.as_ref()?;
        let effort = self.effort.clone()?;
        Some(PromptRequest {
            prompt: self.input.read(cx).value().trim().to_owned().into(),
            generation: GenerationConfig::new(model.provider.clone(), model.id.clone(), effort),
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

    pub(crate) fn model_selector(&self, view: Entity<Self>, _cx: &App) -> AnyElement {
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

        option_button("prompt-model", trigger_label, IconName::Bot)
            .accessibility_id("prompt-model-and-effort-selector")
            .dropdown_menu(move |menu, window, cx| {
                let menu = models.clone().into_iter().fold(
                    menu.min_w(px(230.)).label("Models"),
                    |menu, model| {
                        let select_view = view.clone();
                        let model_for_click = model.clone();
                        menu.item(
                            PopupMenuItem::new(model.display_name.clone())
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
}

impl EventEmitter<PromptComposerEvent> for PromptComposer {}

impl Render for PromptComposer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let _ = &self.subscriptions;
        let focused = self.input.read(cx).focus_handle(cx).is_focused(window);
        let ready = self.is_ready(cx) && !self.generating;
        let submit_view = cx.entity();
        let generating = self.generating;
        let can_add_attachment = self.attachments.len() < MAX_ATTACHMENTS;

        v_flex()
            .w_full()
            .max_w(px(608.))
            .mx_auto()
            .min_h(px(90.))
            .p(px(12.))
            .gap(px(6.))
            .justify_between()
            .rounded(px(22.))
            .border_1()
            .border_color(if focused {
                cx.theme().ring.opacity(0.9)
            } else {
                cx.theme().border.opacity(0.82)
            })
            .bg(cx.theme().popover)
            .shadow_sm()
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
                            .line_height(px(20.)),
                    ),
            )
            .child(
                h_flex()
                    .w_full()
                    .h(px(30.))
                    .items_center()
                    .justify_between()
                    .child(h_flex().items_center().when(can_add_attachment, |this| {
                        this.child(Self::attachment_button(cx))
                    }))
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
    use std::{cell::RefCell, rc::Rc};

    use gpui::{TestAppContext, size};

    use super::*;

    fn model(id: &str, default_effort: EffortLevel) -> ModelDescriptor {
        ModelDescriptor {
            provider: magenta_core::ProviderId::new("openai"),
            id: magenta_core::ModelId::new(id),
            display_name: id.to_owned(),
            description: None,
            priority: 0,
            default_effort,
            supported_efforts: EffortLevel::ALL.to_vec(),
        }
    }

    #[gpui::test]
    fn cancel_is_emitted_only_while_a_response_is_generating(cx: &mut TestAppContext) {
        cx.update(gpui_component::init);
        let window = cx.open_window(size(px(720.), px(420.)), PromptComposer::new);
        let cancellations = Rc::new(RefCell::new(0));
        let observed = Rc::clone(&cancellations);

        let subscription = window
            .update(cx, |composer, _, cx| {
                let subscription = cx.subscribe(&cx.entity(), move |_, _, event, _| {
                    if matches!(event, PromptComposerEvent::Cancel) {
                        *observed.borrow_mut() += 1;
                    }
                });
                composer.cancel(cx);
                composer.set_generating(true, cx);
                composer.cancel(cx);
                subscription
            })
            .expect("the composer test window should remain open");

        cx.run_until_parked();
        assert_eq!(*cancellations.borrow(), 1);
        drop(subscription);
    }

    #[gpui::test]
    fn pending_storage_blocks_submit_and_preserves_newer_draft(cx: &mut TestAppContext) {
        cx.update(gpui_component::init);
        let window = cx.open_window(size(px(900.), px(640.)), PromptComposer::new);
        window
            .update(cx, |composer, window, cx| {
                composer.select_model(model("model", EffortLevel::Medium), cx);
                composer
                    .input
                    .update(cx, |input, cx| input.set_value("First draft", window, cx));
                let submitted = composer.request(cx).unwrap();
                composer.set_storage_ready(false, cx);
                assert!(composer.request(cx).is_none());
                composer
                    .input
                    .update(cx, |input, cx| input.set_value("Newer draft", window, cx));
                composer.clear_submitted(&submitted, window, cx);
                assert_eq!(composer.input.read(cx).value().as_ref(), "Newer draft");
                composer.set_storage_ready(true, cx);
                let submitted = composer.request(cx).unwrap();
                composer.clear_submitted(&submitted, window, cx);
                assert!(composer.input.read(cx).value().is_empty());
            })
            .unwrap();
    }

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
                let model = model("gpt-5.4", EffortLevel::Medium);
                composer.set_models(vec![model.clone()], cx);
                composer.select_model(model, cx);
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
                let model = model("gpt-5.4", EffortLevel::High);
                composer.set_models(vec![model.clone()], cx);
                composer.select_model(model, cx);
                composer.select_effort(EffortLevel::High, cx);

                let request = composer.request(cx).expect("the request should be ready");
                assert_eq!(request.prompt.as_ref(), "A quiet cyan horizon");
                assert_eq!(
                    request.generation.provider,
                    magenta_core::ProviderId::new("openai")
                );
                assert_eq!(
                    request.generation.model,
                    magenta_core::ModelId::new("gpt-5.4")
                );
                assert_eq!(request.generation.effort, EffortLevel::High);
                assert!(request.attachments.is_empty());
            })
            .expect("the composer test window should remain open");
    }

    #[test]
    fn model_options_round_trip_through_core_generation_configuration() {
        let model = model("gpt-5.4", EffortLevel::Medium);
        let configuration =
            GenerationConfig::new(model.provider.clone(), model.id, EffortLevel::Medium);

        assert_eq!(
            configuration.provider,
            magenta_core::ProviderId::new("openai")
        );
        assert_eq!(configuration.model, magenta_core::ModelId::new("gpt-5.4"));
        assert_eq!(configuration.effort, EffortLevel::Medium);
    }

    #[gpui::test]
    fn selecting_a_model_uses_only_its_advertised_efforts(cx: &mut TestAppContext) {
        cx.update(gpui_component::init);
        let window = cx.open_window(size(px(720.), px(420.)), PromptComposer::new);
        let openai = ModelDescriptor {
            supported_efforts: vec![EffortLevel::Low, EffortLevel::Medium, EffortLevel::XHigh],
            default_effort: EffortLevel::XHigh,
            ..model("gpt-5.6-luna", EffortLevel::Medium)
        };
        let gemini_effort = EffortLevel::from_wire("thinking_budget")
            .expect("provider-specific effort should be valid");
        let gemini = ModelDescriptor {
            provider: magenta_core::ProviderId::new("gemini"),
            id: magenta_core::ModelId::new("gemini-2.5-pro"),
            display_name: "Gemini Pro".to_owned(),
            description: None,
            priority: 0,
            default_effort: gemini_effort.clone(),
            supported_efforts: vec![gemini_effort.clone()],
        };

        window
            .update(cx, |composer, _, cx| {
                composer.set_models(vec![openai.clone(), gemini.clone()], cx);
                composer.select_model(openai, cx);
                composer.select_effort(EffortLevel::XHigh, cx);
                assert_eq!(composer.effort, Some(EffortLevel::XHigh));

                composer.select_model(gemini, cx);

                assert_eq!(composer.effort, Some(gemini_effort));
            })
            .expect("the composer test window should remain open");
    }

    #[gpui::test]
    fn core_configuration_restores_the_composer_selection(cx: &mut TestAppContext) {
        cx.update(gpui_component::init);
        let window = cx.open_window(size(px(720.), px(420.)), PromptComposer::new);
        let configuration = GenerationConfig::new(
            magenta_core::ProviderId::new("openai"),
            magenta_core::ModelId::new("gpt-5.4"),
            EffortLevel::High,
        );

        window
            .update(cx, |composer, _, cx| {
                composer.set_models(vec![model("gpt-5.4", EffortLevel::Medium)], cx);
                composer.set_configuration(&configuration, cx);

                assert_eq!(
                    composer.model.as_ref().map(|model| &model.id),
                    Some(&configuration.model)
                );
                assert_eq!(composer.effort, Some(EffortLevel::High));
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
