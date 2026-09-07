use std::{error::Error, future::Future, path::PathBuf, pin::Pin};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkspaceOperation {
    ListFiles {
        path: String,
        depth: u8,
    },
    SearchText {
        query: String,
        path: String,
        glob: Option<String>,
    },
    ReadFile {
        path: String,
        start_line: Option<u32>,
        line_count: Option<u32>,
    },
    ApplyPatch {
        path: String,
        unified_diff: String,
    },
    CreateFile {
        path: String,
        content: String,
    },
}

impl WorkspaceOperation {
    #[must_use]
    pub const fn is_mutating(&self) -> bool {
        matches!(self, Self::ApplyPatch { .. } | Self::CreateFile { .. })
    }

    #[must_use]
    pub fn path(&self) -> &str {
        match self {
            Self::ListFiles { path, .. }
            | Self::SearchText { path, .. }
            | Self::ReadFile { path, .. }
            | Self::ApplyPatch { path, .. }
            | Self::CreateFile { path, .. } => path,
        }
    }

    #[must_use]
    pub const fn tool_name(&self) -> &'static str {
        match self {
            Self::ListFiles { .. } => "list_files",
            Self::SearchText { .. } => "search_text",
            Self::ReadFile { .. } => "read_file",
            Self::ApplyPatch { .. } => "apply_patch",
            Self::CreateFile { .. } => "create_file",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkspaceMutation {
    pub path: String,
    pub expected_digest: Option<String>,
    pub replacement: Vec<u8>,
    pub creates_file: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkspacePreview {
    pub path: String,
    pub summary: String,
    pub output: String,
    pub diff: Option<String>,
    pub protected: bool,
    pub mutation: Option<WorkspaceMutation>,
}

#[derive(Debug, thiserror::Error)]
#[error("workspace operation failed")]
pub struct WorkspaceError {
    #[source]
    pub source: Box<dyn Error + Send + Sync>,
}

impl WorkspaceError {
    #[must_use]
    pub fn new(source: impl Error + Send + Sync + 'static) -> Self {
        Self {
            source: Box::new(source),
        }
    }
}

pub type WorkspaceFuture<T> =
    Pin<Box<dyn Future<Output = Result<T, WorkspaceError>> + Send + 'static>>;

pub trait WorkspaceAccess: Send + Sync {
    fn prepare(
        &self,
        root: PathBuf,
        operation: WorkspaceOperation,
        allow_protected: bool,
    ) -> WorkspaceFuture<WorkspacePreview>;

    fn commit(&self, root: PathBuf, mutation: WorkspaceMutation) -> WorkspaceFuture<String>;
}
