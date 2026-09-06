use std::{
    collections::HashSet,
    fs::{self, OpenOptions},
    io::{Cursor, Write},
    path::{Path, PathBuf},
};

use image::{AnimationDecoder as _, ImageFormat};
use magenta_core::{Attachment, AttachmentDraft, StorageErrorKind};
use rand::RngCore as _;
use rusqlite::Connection;

use crate::{Result, database_error, failure, unavailable};

const MAX_ATTACHMENTS_PER_TURN: usize = 4;
const MAX_ATTACHMENT_BYTES: u64 = 10 * 1024 * 1024;

pub fn import(directory: &Path, drafts: &[AttachmentDraft]) -> Result<Vec<Attachment>> {
    if drafts.len() > MAX_ATTACHMENTS_PER_TURN {
        return Err(failure(
            StorageErrorKind::TooManyAttachments,
            "a turn can contain at most four attachments",
        ));
    }

    let mut attachments = Vec::with_capacity(drafts.len());
    for draft in drafts {
        match import_one(directory, draft) {
            Ok(attachment) => attachments.push(attachment),
            Err(error) => {
                remove_managed(directory, &attachments);
                return Err(error);
            }
        }
    }
    Ok(attachments)
}

pub fn remove_managed(directory: &Path, attachments: &[Attachment]) {
    for attachment in attachments {
        if attachment.managed && attachment.path.starts_with(directory) {
            let _ = fs::remove_file(&attachment.path);
        }
    }
}

pub fn reconcile(directory: &Path, connection: &Connection) -> Result<()> {
    fs::create_dir_all(directory).map_err(unavailable)?;

    let mut statement = connection
        .prepare("SELECT source_path FROM attachments WHERE managed = 1")
        .map_err(database_error)?;
    let rows = statement
        .query_map([], |row| row.get::<_, Vec<u8>>(0))
        .map_err(database_error)?;
    let referenced = rows
        .map(|row| {
            let bytes = row.map_err(database_error)?;
            super::records::decode_path(bytes)
        })
        .collect::<Result<HashSet<_>>>()?;

    for entry in fs::read_dir(directory).map_err(unavailable)? {
        let entry = entry.map_err(unavailable)?;
        let path = entry.path();
        if path.is_file() && !referenced.contains(&path) {
            let _ = fs::remove_file(path);
        }
    }

    Ok(())
}

fn import_one(directory: &Path, draft: &AttachmentDraft) -> Result<Attachment> {
    let metadata = fs::metadata(&draft.source_path).map_err(|_| {
        failure(
            StorageErrorKind::AttachmentUnreadable,
            "cannot read attachment",
        )
    })?;
    if !metadata.is_file() {
        return Err(failure(
            StorageErrorKind::AttachmentUnreadable,
            "attachment is not a regular file",
        ));
    }
    if metadata.len() > MAX_ATTACHMENT_BYTES {
        return Err(failure(
            StorageErrorKind::AttachmentTooLarge,
            "attachment exceeds the ten mebibyte limit",
        ));
    }

    let bytes = fs::read(&draft.source_path).map_err(|_| {
        failure(
            StorageErrorKind::AttachmentUnreadable,
            "cannot read attachment",
        )
    })?;
    let byte_size = u64::try_from(bytes.len()).map_err(|_| {
        failure(
            StorageErrorKind::AttachmentTooLarge,
            "attachment exceeds the ten mebibyte limit",
        )
    })?;
    if byte_size > MAX_ATTACHMENT_BYTES {
        return Err(failure(
            StorageErrorKind::AttachmentTooLarge,
            "attachment exceeds the ten mebibyte limit",
        ));
    }

    let (mime_type, extension) = image_type(&bytes)?;
    let destination = copy_atomically(directory, &bytes, extension)?;

    Ok(Attachment {
        name: draft.name.clone(),
        path: destination,
        mime_type: mime_type.to_owned(),
        byte_size,
        managed: true,
    })
}

fn image_type(bytes: &[u8]) -> Result<(&'static str, &'static str)> {
    let format = image::guess_format(bytes).map_err(|_| {
        failure(
            StorageErrorKind::UnsupportedAttachment,
            "attachment is not a supported image",
        )
    })?;

    match format {
        ImageFormat::Png => Ok(("image/png", "png")),
        ImageFormat::Jpeg => Ok(("image/jpeg", "jpg")),
        ImageFormat::WebP => Ok(("image/webp", "webp")),
        ImageFormat::Gif => {
            let decoder =
                image::codecs::gif::GifDecoder::new(Cursor::new(bytes)).map_err(|_| {
                    failure(
                        StorageErrorKind::UnsupportedAttachment,
                        "attachment is not a readable GIF image",
                    )
                })?;
            let frames = decoder.into_frames().collect_frames().map_err(|_| {
                failure(
                    StorageErrorKind::UnsupportedAttachment,
                    "attachment is not a readable GIF image",
                )
            })?;
            if frames.len() > 1 {
                return Err(failure(
                    StorageErrorKind::AnimatedImage,
                    "animated GIF images are not supported",
                ));
            }
            Ok(("image/gif", "gif"))
        }
        _ => Err(failure(
            StorageErrorKind::UnsupportedAttachment,
            "attachment is not a supported image",
        )),
    }
}

fn copy_atomically(directory: &Path, bytes: &[u8], extension: &str) -> Result<PathBuf> {
    fs::create_dir_all(directory).map_err(unavailable)?;

    for _ in 0..16 {
        let name = opaque_name(extension);
        let destination = directory.join(&name);
        let temporary = directory.join(format!(".{name}.partial"));

        let mut output = match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
        {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(unavailable(error)),
        };
        let copied = (|| {
            output.write_all(bytes).map_err(unavailable)?;
            output.flush().map_err(unavailable)?;
            output.sync_all().map_err(unavailable)?;
            Ok(())
        })();
        drop(output);

        match copied {
            Ok(()) => match fs::rename(&temporary, &destination).map_err(unavailable) {
                Ok(()) => return Ok(destination),
                Err(error) => {
                    let _ = fs::remove_file(&temporary);
                    return Err(error);
                }
            },
            Err(error) => {
                let _ = fs::remove_file(&temporary);
                return Err(error);
            }
        }
    }

    Err(failure(
        StorageErrorKind::Unavailable,
        "could not allocate an attachment filename",
    ))
}

fn opaque_name(extension: &str) -> String {
    let mut bytes = [0_u8; 16];
    rand::rng().fill_bytes(&mut bytes);
    let mut name = String::with_capacity(33 + extension.len());
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(name, "{byte:02x}");
    }
    name.push('.');
    name.push_str(extension);
    name
}
