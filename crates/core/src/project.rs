//! Registered projects and read-only workspace browsing ports.

use std::path::PathBuf;

use crate::{StorageFuture, Timestamp, WorkspaceFuture};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Project {
    pub name: String,
    pub root: PathBuf,
    pub added_at: Timestamp,
    pub last_opened_at: Timestamp,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkspaceEntryKind {
    Directory,
    File,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkspaceEntry {
    pub path: String,
    pub name: String,
    pub kind: WorkspaceEntryKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkspaceDocument {
    pub path: String,
    pub content: String,
    pub language: String,
}

pub trait ProjectStore: Send + Sync {
    fn projects(&self) -> StorageFuture<Vec<Project>>;
    fn upsert_project(&self, project: Project) -> StorageFuture<()>;
    fn remove_project(&self, root: PathBuf) -> StorageFuture<()>;
}

pub trait WorkspaceBrowser: Send + Sync {
    fn canonicalize_root(&self, root: PathBuf) -> WorkspaceFuture<PathBuf>;
    fn list_directory(&self, root: PathBuf, path: String) -> WorkspaceFuture<Vec<WorkspaceEntry>>;
    fn read_document(&self, root: PathBuf, path: String) -> WorkspaceFuture<WorkspaceDocument>;
}
