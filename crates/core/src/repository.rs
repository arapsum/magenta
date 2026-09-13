//! Git repository values and the asynchronous repository access port.

use std::{error::Error, future::Future, path::PathBuf, pin::Pin};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RepositoryChangeKind {
    Added,
    Modified,
    Deleted,
    Renamed,
    Untracked,
    Conflicted,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RepositoryChange {
    pub path: String,
    pub original_path: Option<String>,
    pub staged: Option<RepositoryChangeKind>,
    pub unstaged: Option<RepositoryChangeKind>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RepositoryStatus {
    pub branch: Option<String>,
    pub detached: bool,
    pub unborn: bool,
    pub changes: Vec<RepositoryChange>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RepositoryDiffArea {
    Staged,
    Unstaged,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RepositoryDiff {
    pub path: String,
    pub area: RepositoryDiffArea,
    pub unified_diff: String,
    pub binary: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RepositoryCommit {
    pub oid: String,
    pub summary: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RepositoryErrorKind {
    GitUnavailable,
    NotRepository,
    WorkspaceIsNotRepositoryRoot,
    InvalidPath,
    Conflict,
    Other,
}

#[derive(Debug, thiserror::Error)]
#[error("repository operation failed ({kind:?})")]
pub struct RepositoryError {
    pub kind: RepositoryErrorKind,
    #[source]
    pub source: Box<dyn Error + Send + Sync>,
}

impl RepositoryError {
    #[must_use]
    pub fn new(kind: RepositoryErrorKind, source: impl Error + Send + Sync + 'static) -> Self {
        Self {
            kind,
            source: Box::new(source),
        }
    }
}

pub type RepositoryFuture<T> =
    Pin<Box<dyn Future<Output = Result<T, RepositoryError>> + Send + 'static>>;

pub trait RepositoryAccess: Send + Sync {
    fn status(&self, root: PathBuf) -> RepositoryFuture<RepositoryStatus>;
    fn diff(
        &self,
        root: PathBuf,
        path: String,
        area: RepositoryDiffArea,
    ) -> RepositoryFuture<RepositoryDiff>;
    fn stage(&self, root: PathBuf, paths: Vec<String>) -> RepositoryFuture<()>;
    fn unstage(&self, root: PathBuf, paths: Vec<String>) -> RepositoryFuture<()>;
    fn commit(&self, root: PathBuf, message: String) -> RepositoryFuture<RepositoryCommit>;
}
