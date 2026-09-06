use std::{
    fs,
    path::{Path, PathBuf},
};

use image::{Frame, RgbaImage, codecs::gif::GifEncoder};
use magenta_core::{
    AttachmentDraft, BeginTurn, ConversationId, ConversationStore, EffortLevel, GenerationConfig,
    ModelId, ProviderId, StorageErrorKind,
};
use magenta_storage::SqliteConversationStore;

const PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4,
    0x89, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x44, 0x41, 0x54, 0x08, 0x99, 0x63, 0xf8, 0xcf, 0xc0, 0xf0,
    0x1f, 0x00, 0x05, 0x00, 0x01, 0xff, 0x89, 0x99, 0x3d, 0x1d, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45,
    0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];

fn begin_turn(attachments: Vec<AttachmentDraft>) -> BeginTurn {
    BeginTurn {
        conversation_id: None,
        title: "Image attachment".into(),
        prompt: "What is in this image?".into(),
        attachments,
        generation: GenerationConfig::new(
            ProviderId::new("openai"),
            ModelId::new("test-model"),
            EffortLevel::Medium,
        ),
    }
}

fn draft(path: PathBuf) -> AttachmentDraft {
    AttachmentDraft {
        name: "submitted-image.png".into(),
        source_path: path,
    }
}

fn write_png(directory: &Path, name: &str) -> PathBuf {
    let path = directory.join(name);
    fs::write(&path, PNG).unwrap();
    path
}

fn attachments_directory(database_path: &Path) -> PathBuf {
    database_path.parent().unwrap().join("attachments")
}

#[test]
fn imported_images_survive_source_removal_and_restart() {
    smol::block_on(async {
        let directory = tempfile::tempdir().unwrap();
        let database_path = directory.path().join("history.sqlite3");
        let source = write_png(directory.path(), "camera-output.bin");
        let store = SqliteConversationStore::new(database_path.clone());
        store.initialize().await.unwrap();

        let prepared = store
            .begin_turn(begin_turn(vec![draft(source.clone())]))
            .await
            .unwrap();
        let imported = prepared.user_message.attachments[0].clone();
        assert_eq!(imported.name, "submitted-image.png");
        assert_eq!(imported.mime_type, "image/png");
        assert_eq!(imported.byte_size, u64::try_from(PNG.len()).unwrap());
        assert!(imported.managed);
        assert!(
            imported
                .path
                .starts_with(attachments_directory(&database_path))
        );
        assert!(imported.path.exists());
        assert_eq!(prepared.context, vec![prepared.user_message.clone()]);

        fs::remove_file(source).unwrap();
        drop(store);

        let reopened = SqliteConversationStore::new(database_path);
        reopened.initialize().await.unwrap();
        let loaded = reopened.load(prepared.conversation.id).await.unwrap();
        assert_eq!(loaded.page.messages[0].message.attachments, vec![imported]);
    });
}

#[test]
fn failed_persistence_removes_freshly_imported_images() {
    smol::block_on(async {
        let directory = tempfile::tempdir().unwrap();
        let database_path = directory.path().join("history.sqlite3");
        let source = write_png(directory.path(), "source.png");
        let store = SqliteConversationStore::new(database_path.clone());
        store.initialize().await.unwrap();
        let connection = rusqlite::Connection::open(&database_path).unwrap();
        connection
            .execute_batch(
                r"
                    CREATE TRIGGER reject_assistant
                    BEFORE INSERT ON messages
                    WHEN NEW.role = 'assistant'
                    BEGIN
                        SELECT RAISE(ABORT, 'test write failure');
                    END;
                ",
            )
            .unwrap();

        assert!(
            store
                .begin_turn(begin_turn(vec![draft(source)]))
                .await
                .is_err()
        );
        assert!(
            fs::read_dir(attachments_directory(&database_path))
                .unwrap()
                .next()
                .is_none()
        );
    });
}

#[test]
fn attachments_enforce_count_size_and_format_limits() {
    smol::block_on(async {
        let directory = tempfile::tempdir().unwrap();
        let database_path = directory.path().join("history.sqlite3");
        let source = write_png(directory.path(), "source.png");
        let store = SqliteConversationStore::new(database_path);
        store.initialize().await.unwrap();

        let many = std::iter::repeat_with(|| draft(source.clone()))
            .take(5)
            .collect();
        assert_eq!(
            store
                .begin_turn(begin_turn(many))
                .await
                .err()
                .expect("too many attachments must fail")
                .kind,
            StorageErrorKind::TooManyAttachments
        );

        let large = directory.path().join("large.png");
        fs::write(&large, vec![0_u8; 10 * 1024 * 1024 + 1]).unwrap();
        assert_eq!(
            store
                .begin_turn(begin_turn(vec![draft(large)]))
                .await
                .err()
                .expect("oversized attachment must fail")
                .kind,
            StorageErrorKind::AttachmentTooLarge
        );

        let text = directory.path().join("not-an-image.png");
        fs::write(&text, "plain text").unwrap();
        assert_eq!(
            store
                .begin_turn(begin_turn(vec![draft(text)]))
                .await
                .err()
                .expect("unsupported attachment must fail")
                .kind,
            StorageErrorKind::UnsupportedAttachment
        );

        assert_eq!(
            store
                .begin_turn(begin_turn(vec![draft(
                    directory.path().join("missing.png")
                )]))
                .await
                .err()
                .expect("missing attachment must fail")
                .kind,
            StorageErrorKind::AttachmentUnreadable
        );
    });
}

#[test]
fn animated_gifs_are_rejected() {
    smol::block_on(async {
        let directory = tempfile::tempdir().unwrap();
        let database_path = directory.path().join("history.sqlite3");
        let source = directory.path().join("animated.gif");
        let mut encoder = GifEncoder::new(fs::File::create(&source).unwrap());
        encoder
            .encode_frame(Frame::new(RgbaImage::new(1, 1)))
            .unwrap();
        encoder
            .encode_frame(Frame::new(RgbaImage::new(1, 1)))
            .unwrap();
        drop(encoder);

        let store = SqliteConversationStore::new(database_path);
        store.initialize().await.unwrap();
        assert_eq!(
            store
                .begin_turn(begin_turn(vec![draft(source)]))
                .await
                .err()
                .expect("animated GIF must fail")
                .kind,
            StorageErrorKind::AnimatedImage
        );
    });
}

#[test]
fn version_one_attachments_remain_unmanaged_and_their_sources_are_not_deleted() {
    smol::block_on(async {
        let directory = tempfile::tempdir().unwrap();
        let database_path = directory.path().join("history.sqlite3");
        let legacy_source = write_png(directory.path(), "legacy-source.png");
        create_version_one_database(&database_path, &legacy_source);

        let store = SqliteConversationStore::new(database_path.clone());
        store.initialize().await.unwrap();
        let loaded = store.load(ConversationId(1)).await.unwrap();
        let attachment = &loaded.page.messages[0].message.attachments[0];
        assert_eq!(attachment.path, legacy_source);
        assert_eq!(attachment.mime_type, "application/octet-stream");
        assert_eq!(attachment.byte_size, 0);
        assert!(!attachment.managed);

        store.delete(ConversationId(1)).await.unwrap();
        assert!(legacy_source.exists());

        let connection = rusqlite::Connection::open(database_path).unwrap();
        let version: i64 = connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, 2);
    });
}

#[test]
fn initialization_removes_orphaned_attachment_files() {
    smol::block_on(async {
        let directory = tempfile::tempdir().unwrap();
        let database_path = directory.path().join("history.sqlite3");
        let attachments = attachments_directory(&database_path);
        fs::create_dir_all(&attachments).unwrap();
        let orphan = attachments.join(".unfinished.partial");
        fs::write(&orphan, PNG).unwrap();

        let store = SqliteConversationStore::new(database_path);
        store.initialize().await.unwrap();
        assert!(!orphan.exists());
    });
}

fn create_version_one_database(database_path: &Path, source: &Path) {
    let connection = rusqlite::Connection::open(database_path).unwrap();
    connection
        .execute_batch(
            r"
                CREATE TABLE conversations (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    title TEXT NOT NULL,
                    generation TEXT NOT NULL,
                    pinned INTEGER NOT NULL DEFAULT 0 CHECK (pinned IN (0, 1)),
                    created_at INTEGER NOT NULL,
                    updated_at INTEGER NOT NULL
                );
                CREATE TABLE messages (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    conversation_id INTEGER NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
                    sequence INTEGER NOT NULL CHECK (sequence >= 0),
                    role TEXT NOT NULL CHECK (role IN ('user', 'assistant')),
                    content TEXT NOT NULL,
                    status TEXT NOT NULL CHECK (status IN ('complete', 'streaming', 'stopped', 'failed')),
                    generation TEXT NOT NULL,
                    outcome TEXT,
                    created_at INTEGER NOT NULL,
                    UNIQUE (conversation_id, sequence)
                );
                CREATE TABLE attachments (
                    message_id INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
                    position INTEGER NOT NULL,
                    name TEXT NOT NULL,
                    source_path BLOB NOT NULL,
                    PRIMARY KEY (message_id, position)
                );
                PRAGMA user_version = 1;
            ",
        )
        .unwrap();
    let generation = serde_json::to_string(&GenerationConfig::new(
        ProviderId::new("openai"),
        ModelId::new("test-model"),
        EffortLevel::Medium,
    ))
    .unwrap();
    connection
        .execute(
            r"
                INSERT INTO conversations(id, title, generation, created_at, updated_at)
                VALUES (1, 'Legacy attachment', ?1, 0, 0)
            ",
            [&generation],
        )
        .unwrap();
    connection
        .execute(
            r"
                INSERT INTO messages(
                    id, conversation_id, sequence, role, content, status, generation, created_at
                )
                VALUES (1, 1, 0, 'user', 'Legacy', 'complete', ?1, 0)
            ",
            [&generation],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO attachments(message_id, position, name, source_path) VALUES (1, 0, ?1, ?2)",
            rusqlite::params!["legacy-source.png", encoded_path(source)],
        )
        .unwrap();
}

#[cfg(unix)]
fn encoded_path(path: &Path) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt as _;

    path.as_os_str().as_bytes().to_vec()
}

#[cfg(windows)]
fn encoded_path(path: &Path) -> Vec<u8> {
    use std::os::windows::ffi::OsStrExt as _;

    path.as_os_str()
        .encode_wide()
        .flat_map(u16::to_le_bytes)
        .collect()
}
