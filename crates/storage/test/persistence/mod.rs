use magenta_core::{
    AttachmentDraft, BeginTurn, ConversationId, ConversationMode, ConversationStore, EffortLevel,
    FinishReason, GenerationConfig, GenerationOutcome, MessageFailure, MessageFailureCategory,
    MessageFailureDetail, MessageSequence, MessageStatus, ModelId, Project, ProjectStore,
    ProviderId, StorageErrorKind, Timestamp, TokenUsage,
};
use magenta_storage::SqliteConversationStore;
use std::{
    fs,
    path::{Path, PathBuf},
};

mod attachments;
mod history;
mod migration;
mod paging;
mod projects;
mod recovery;

const PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4,
    0x89, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x41, 0x54, 0x08, 0x99, 0x63, 0xf8, 0xcf, 0xc0,
    0xf0, 0x1f, 0x00, 0x05, 0x00, 0x01, 0xff, 0x89, 0x99, 0x3d, 0x1d, 0x00, 0x00, 0x00, 0x00, 0x49,
    0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];

fn input(id: Option<ConversationId>) -> BeginTurn {
    BeginTurn {
        conversation_id: id,
        title: "Unicode λ and Markdown".into(),
        prompt: "Explain `λ`\n```rust\nfn main() {}\n```".into(),
        attachments: Vec::new(),
        generation: GenerationConfig::new(
            ProviderId::new("openai"),
            ModelId::new("test-model"),
            EffortLevel::Custom {
                value: "budget".into(),
                label: "Thinking budget".into(),
            },
        ),
        mode: ConversationMode::Chat,
        workspace_root: None,
        request_overhead_tokens: 0,
    }
}

fn input_with_attachments(
    id: Option<ConversationId>,
    attachments: Vec<AttachmentDraft>,
) -> BeginTurn {
    BeginTurn {
        attachments,
        ..input(id)
    }
}

fn write_png(directory: &Path, name: &str) -> PathBuf {
    let path = directory.join(name);
    fs::write(&path, PNG).unwrap();
    path
}
