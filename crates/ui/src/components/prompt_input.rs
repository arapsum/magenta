use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
};

use gpui::{
    App, AppContext as _, Context, Entity, EventEmitter, Focusable as _, InteractiveElement as _,
    IntoElement, ObjectFit, ParentElement as _, PathPromptOptions, Render, SharedString,
    Styled as _, StyledImage as _, Subscription, Task, Window, div, img, linear_color_stop,
    linear_gradient, prelude::FluentBuilder as _, px,
};
use gpui_component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _, WindowExt,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{InputEvent, Textarea, TextareaState},
    menu::{DropdownMenu as _, PopupMenuItem},
    notification::{Notification, NotificationType},
    v_flex,
};

use crate::{MagentaError, notification_for_error};

const MAX_ATTACHMENTS: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageModel {
    NanoBananaPro,
    NanoBanana,
    Imagen4,
}

impl ImageModel {
    const ALL: [Self; 3] = [Self::NanoBananaPro, Self::NanoBanana, Self::Imagen4];

    const fn label(self) -> &'static str {
        match self {
            Self::NanoBananaPro => "Nano Banana Pro",
            Self::NanoBanana => "Nano Banana",
            Self::Imagen4 => "Imagen 4",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EffortLevel {
    Low,
    Medium,
    High,
}

impl EffortLevel {
    const ALL: [Self; 3] = [Self::Low, Self::Medium, Self::High];

    const fn label(self) -> &'static str {
        match self {
            Self::Low => "Low",
            Self::Medium => "Medium",
            Self::High => "High",
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
    pub model: ImageModel,
    pub effort: EffortLevel,
    pub attachments: Vec<PathBuf>,
}

#[derive(Clone, Debug)]
pub enum PromptComposerEvent {
    Submit(PromptRequest),
}

pub struct PromptComposer {
    input: Entity<TextareaState>,
    model: Option<ImageModel>,
    effort: Option<EffortLevel>,
    attachments: Vec<ReferenceImage>,
    attachment_task: Option<Task<()>>,
    subscriptions: Vec<Subscription>,
}

impl PromptComposer {
    pub fn new(window: &mut Window, cx: &mut Context<'_, Self>) -> Self {
        let input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("Describe a new image")
                .auto_grow(2, 5)
                .submit_on_enter(true)
        });

        let subscriptions = vec![cx.subscribe_in(
            &input,
            window,
            |composer, _, event: &InputEvent, window, cx| match event {
                InputEvent::Change | InputEvent::Focus | InputEvent::Blur => cx.notify(),
                InputEvent::PressEnter { shift: false, .. } => composer.submit(window, cx),
                InputEvent::PressEnter { shift: true, .. } => {}
            },
        )];

        Self {
            input,
            model: None,
            effort: None,
            attachments: Vec::new(),
            attachment_task: None,
            subscriptions,
        }
    }

    pub fn focus(&self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.input.focus_handle(cx).focus(window, cx);
    }

    fn has_content(&self, cx: &App) -> bool {
        !self.input.read(cx).value().trim().is_empty() || !self.attachments.is_empty()
    }

    fn is_ready(&self, cx: &App) -> bool {
        self.has_content(cx) && self.model.is_some() && self.effort.is_some()
    }

    fn select_model(&mut self, model: ImageModel, cx: &mut Context<'_, Self>) {
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
                    .message("Remove an image before adding another reference.")
                    .with_type(NotificationType::Warning),
                cx,
            );
            return;
        }

        let picker = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Add reference images".into()),
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

    fn submit(&self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(request) = self.request(cx) else {
            return;
        };

        cx.emit(PromptComposerEvent::Submit(request));
        window.push_notification(
            Notification::new()
                .title("Prompt ready")
                .message("Generation will begin when an image service is connected.")
                .with_type(NotificationType::Info),
            cx,
        );
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
                        .accessibility_id("prompt-add-reference-image")
                        .tooltip("Add reference images")
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
                let menu = ImageModel::ALL.into_iter().fold(
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
}

impl EventEmitter<PromptComposerEvent> for PromptComposer {}

impl Render for PromptComposer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let _ = &self.subscriptions;
        let focused = self.input.read(cx).focus_handle(cx).is_focused(window);
        let ready = self.is_ready(cx);
        let submit_view = cx.entity();

        v_flex()
            .w_full()
            .min_h(px(145.))
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
                    .child(
                        Textarea::new(&self.input)
                            .appearance(false)
                            .bordered(false)
                            .aria_label("Image generation prompt")
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
                        Button::new("generate")
                            .primary()
                            .disabled(!ready)
                            .accessibility_id("prompt-generate")
                            .tooltip(if ready {
                                "Generate image"
                            } else {
                                "Add a prompt or image, then choose a model and effort"
                            })
                            .size(px(36.))
                            .p_0()
                            .rounded_full()
                            .icon(IconName::ChevronUp)
                            .on_click(move |_, window, cx| {
                                submit_view.update(cx, |composer, cx| {
                                    composer.submit(window, cx);
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
                composer.select_model(ImageModel::NanoBananaPro, cx);
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
                composer.select_model(ImageModel::Imagen4, cx);
                composer.select_effort(EffortLevel::High, cx);

                let request = composer.request(cx).expect("the request should be ready");
                assert_eq!(request.prompt.as_ref(), "A quiet cyan horizon");
                assert_eq!(request.model, ImageModel::Imagen4);
                assert_eq!(request.effort, EffortLevel::High);
                assert!(request.attachments.is_empty());
            })
            .expect("the composer test window should remain open");
    }
}
