//! Project registration and read-only workspace browsing workflows.

use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use magenta_core::{
    Project, ProjectStore, StorageError, StorageErrorKind, WorkspaceBrowser, WorkspaceDocument,
    WorkspaceEntry, WorkspaceError,
};

#[derive(Clone)]
pub struct ProjectCatalog {
    store: Arc<dyn ProjectStore>,
    browser: Arc<dyn WorkspaceBrowser>,
}

impl ProjectCatalog {
    #[must_use]
    pub fn new(store: Arc<dyn ProjectStore>, browser: Arc<dyn WorkspaceBrowser>) -> Self {
        Self { store, browser }
    }

    /// Lists the projects registered for the current profile.
    ///
    /// # Errors
    ///
    /// Returns the storage error when the project registry cannot be read.
    pub async fn list(&self) -> Result<Vec<Project>, StorageError> {
        self.store.projects().await
    }

    /// Canonicalizes and registers a workspace as a project.
    ///
    /// # Errors
    ///
    /// Returns an error when the workspace cannot be canonicalized or the
    /// project cannot be persisted.
    pub async fn add(&self, root: PathBuf) -> Result<Project, ProjectCatalogError> {
        let root = self.browser.canonicalize_root(root).await?;
        let name = project_name(&root);
        let timestamp = timestamp()?;
        let project = Project {
            name,
            root,
            added_at: timestamp,
            last_opened_at: timestamp,
        };
        self.store.upsert_project(project.clone()).await?;
        Ok(project)
    }

    /// Updates the last-opened timestamp for a registered project.
    ///
    /// # Errors
    ///
    /// Returns the storage error when the timestamp cannot be created or the
    /// project cannot be persisted.
    pub async fn touch(&self, mut project: Project) -> Result<Project, ProjectCatalogError> {
        project.last_opened_at = timestamp()?;
        self.store.upsert_project(project.clone()).await?;
        Ok(project)
    }

    /// Removes a project registration without touching its conversations.
    ///
    /// # Errors
    ///
    /// Returns the storage error when the registry cannot be updated.
    pub async fn forget(&self, root: PathBuf) -> Result<(), StorageError> {
        self.store.remove_project(root).await
    }

    /// Lists the direct children of a project directory.
    ///
    /// # Errors
    ///
    /// Returns the workspace error when the directory cannot be inspected.
    pub async fn entries(
        &self,
        root: PathBuf,
        path: String,
    ) -> Result<Vec<WorkspaceEntry>, WorkspaceError> {
        self.browser.list_directory(root, path).await
    }

    /// Reads one bounded, supported document from a project.
    ///
    /// # Errors
    ///
    /// Returns the workspace error when the path is unsafe, unsupported, or
    /// cannot be read.
    pub async fn document(
        &self,
        root: PathBuf,
        path: String,
    ) -> Result<WorkspaceDocument, WorkspaceError> {
        self.browser.read_document(root, path).await
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ProjectCatalogError {
    #[error(transparent)]
    Storage(#[from] StorageError),
    #[error(transparent)]
    Workspace(#[from] WorkspaceError),
}

fn project_name(root: &Path) -> String {
    root.file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .map_or_else(|| root.display().to_string(), str::to_owned)
}

fn timestamp() -> Result<magenta_core::Timestamp, StorageError> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(invalid)?
        .as_millis();
    let millis = i64::try_from(millis).map_err(invalid)?;
    Ok(magenta_core::Timestamp(millis))
}

fn invalid(error: impl std::error::Error + Send + Sync + 'static) -> StorageError {
    StorageError::new(StorageErrorKind::InvalidData, error)
}
