use std::path::{Path, PathBuf};

use gpui_kit::{Context, PathPromptOptions, Window};

use super::state::{PromptComposer, ReferenceImage};
use super::{MAX_ATTACHMENT_BYTES, MAX_ATTACHMENTS};
use crate::{ErrorPresentation, MagentaError};

impl PromptComposer {
    pub(super) fn choose_attachments(&mut self, window: &Window, cx: &mut Context<'_, Self>) {
        if self.attachments.len() >= MAX_ATTACHMENTS {
            self.inline_error = Some(ErrorPresentation {
                code: "MAG-ATTACHMENT-COUNT",
                severity: crate::ErrorSeverity::Warning,
                title: "Four images already attached",
                message: "Remove an image before adding another attachment.",
            });
            cx.notify();
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
                    _ = composer.update_in(window, |composer, _window, cx| {
                        composer.attachment_task = None;
                        let error = MagentaError::AttachmentPicker { source };
                        composer.set_inline_error(error.presentation(), cx);
                    });
                    return;
                }
                Err(_) => return,
            };
            let Some(paths) = selection else {
                _ = composer.update_in(window, |composer, _, _| composer.attachment_task = None);
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
        _window: &mut Window,
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
            } else if !readable {
                unreadable += 1;
            } else if byte_size.is_none_or(|size| size > MAX_ATTACHMENT_BYTES) {
                too_large += 1;
            } else if self
                .attachments
                .iter()
                .any(|attachment| attachment.path == path)
            {
                duplicates += 1;
            } else if self.attachments.len() >= MAX_ATTACHMENTS {
                overflow += 1;
            } else {
                self.attachments.push(ReferenceImage::new(path));
            }
        }

        let skipped = unsupported + unreadable + too_large + duplicates + overflow;
        if skipped > 0 {
            let _ = (unsupported, unreadable, too_large, duplicates, overflow);
            self.inline_error = Some(ErrorPresentation {
                code: "MAG-ATTACHMENT-SKIPPED",
                severity: crate::ErrorSeverity::Warning,
                title: "Some images were not added",
                message: "Some selected images were unsupported, unreadable, too large, duplicated, or beyond the four-image limit.",
            });
        }
        cx.notify();
    }

    pub(super) fn remove_attachment(&mut self, path: &Path, cx: &mut Context<'_, Self>) {
        let before = self.attachments.len();
        self.attachments
            .retain(|attachment| attachment.path != *path);
        if self.attachments.len() != before {
            cx.notify();
        }
    }
}

pub(super) fn is_supported_image(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "png" | "jpg" | "jpeg" | "webp"
            )
        })
}
