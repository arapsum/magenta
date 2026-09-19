use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

use agentfs_sdk::{
    AgentFS, AgentFSOptions,
    filesystem::{FileSystem, HostFS, OverlayFS},
};
use magenta_core::{
    WorkspaceAccess, WorkspaceError, WorkspaceFuture, WorkspaceMutation, WorkspaceOperation,
    WorkspacePreview, WorkspaceSessionAccess,
};
use parking_lot::{Mutex, RwLock};

use crate::{
    operations::{LocalWorkspace, digest, display_diff},
    patch::apply_unified,
    path,
};

struct Session {
    id: String,
    database_path: PathBuf,
    filesystem: Arc<AgentFS>,
    overlay: Arc<OverlayFS>,
    original_digests: Mutex<HashMap<String, Option<String>>>,
}

#[derive(Clone)]
pub struct AgentFsWorkspace {
    sessions_directory: Arc<PathBuf>,
    sessions: Arc<RwLock<HashMap<PathBuf, Arc<Session>>>>,
    runtime: Arc<tokio::runtime::Runtime>,
    fallback: LocalWorkspace,
}

impl AgentFsWorkspace {
    /// Creates the local `AgentFS` adapter.
    ///
    /// Failure means Work mode must remain read-only.
    ///
    /// # Errors
    ///
    /// Returns an error when the local Tokio runtime cannot be created.
    pub fn new(sessions_directory: PathBuf) -> Result<Self, WorkspaceError> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .worker_threads(2)
            .build()
            .map_err(WorkspaceError::new)?;

        Ok(Self {
            sessions_directory: Arc::new(sessions_directory),
            sessions: Arc::new(RwLock::new(HashMap::new())),
            runtime: Arc::new(runtime),
            fallback: LocalWorkspace,
        })
    }

    fn session(&self, root: &Path) -> Option<Arc<Session>> {
        let root = path::canonical_root(root).ok()?;

        self.sessions.read().get(&root).cloned()
    }
}

impl WorkspaceSessionAccess for AgentFsWorkspace {
    fn ensure_session_available(&self, root: PathBuf) -> WorkspaceFuture<()> {
        let this = self.clone();
        Box::pin(smol::unblock(move || {
            let root = path::canonical_root(&root).map_err(WorkspaceError::new)?;
            let previous = this.sessions.read().get(&root).cloned();
            if let Some(previous) = previous {
                let has_changes = !this
                    .runtime
                    .block_on(previous.filesystem.get_delta_paths())
                    .map_err(WorkspaceError::new)?
                    .is_empty();
                let detail = if has_changes {
                    "workspace has staged changes awaiting review"
                } else {
                    "workspace already has an active Work run"
                };
                return Err(WorkspaceError::new(std::io::Error::other(detail)));
            }
            Ok(())
        }))
    }

    fn start_session(&self, root: PathBuf, session_id: String) -> WorkspaceFuture<PathBuf> {
        let this = self.clone();
        Box::pin(smol::unblock(move || {
            let root = path::canonical_root(&root).map_err(WorkspaceError::new)?;

            let previous = this.sessions.read().get(&root).cloned();
            if let Some(previous) = previous {
                let has_changes = !this
                    .runtime
                    .block_on(previous.filesystem.get_delta_paths())
                    .map_err(WorkspaceError::new)?
                    .is_empty();
                let detail = if has_changes {
                    "workspace has staged changes awaiting review"
                } else {
                    "workspace already has an active Work run"
                };
                return Err(WorkspaceError::new(std::io::Error::other(detail)));
            }

            std::fs::create_dir_all(this.sessions_directory.as_path())
                .map_err(WorkspaceError::new)?;

            let database_path = this.sessions_directory.join(format!("{session_id}.db"));
            let options = AgentFSOptions::with_path(database_path.to_string_lossy().into_owned())
                .with_base(root.clone());

            let filesystem = this
                .runtime
                .block_on(AgentFS::open(options))
                .map_err(WorkspaceError::new)?;

            let host = Arc::new(HostFS::new(root.clone()).map_err(WorkspaceError::new)?);
            let overlay = Arc::new(OverlayFS::new(host, filesystem.fs.clone()));
            this.runtime
                .block_on(overlay.load())
                .map_err(WorkspaceError::new)?;

            this.sessions.write().insert(
                root,
                Arc::new(Session {
                    id: session_id,
                    database_path: database_path.clone(),
                    filesystem: Arc::new(filesystem),
                    overlay,
                    original_digests: Mutex::new(HashMap::new()),
                }),
            );
            Ok(database_path)
        }))
    }

    fn session_has_changes(&self, root: PathBuf, session_id: String) -> WorkspaceFuture<bool> {
        let this = self.clone();
        Box::pin(smol::unblock(move || {
            let root = path::canonical_root(&root).map_err(WorkspaceError::new)?;
            let session = this.sessions.read().get(&root).cloned().ok_or_else(|| {
                WorkspaceError::new(std::io::Error::other("no active AgentFS session"))
            })?;
            if session.id != session_id {
                return Err(WorkspaceError::new(std::io::Error::other(
                    "AgentFS session does not match",
                )));
            }
            Ok(!this
                .runtime
                .block_on(session.filesystem.get_delta_paths())
                .map_err(WorkspaceError::new)?
                .is_empty())
        }))
    }

    fn apply_session(&self, root: PathBuf, session_id: String) -> WorkspaceFuture<Vec<String>> {
        let this = self.clone();
        Box::pin(smol::unblock(move || {
            let root = path::canonical_root(&root).map_err(WorkspaceError::new)?;

            let session = this.sessions.read().get(&root).cloned().ok_or_else(|| {
                WorkspaceError::new(std::io::Error::other("no active AgentFS session"))
            })?;

            if session.id != session_id {
                return Err(WorkspaceError::new(std::io::Error::other(
                    "AgentFS session does not match",
                )));
            }

            let paths = this
                .runtime
                .block_on(session.filesystem.get_delta_paths())
                .map_err(WorkspaceError::new)?;
            let mut paths = paths.into_iter().collect::<Vec<_>>();
            paths.sort_by_key(|path| (path.matches('/').count(), path.clone()));

            let originals = session.original_digests.lock().clone();

            for (relative, expected) in &originals {
                let target =
                    path::safe_path(&root, relative, false).map_err(WorkspaceError::new)?;
                let actual = std::fs::read(&target).ok().map(|bytes| digest(&bytes));
                if &actual != expected {
                    return Err(WorkspaceError::new(std::io::Error::other(format!(
                        "{relative} changed after the AgentFS session started"
                    ))));
                }
            }

            let mut changes = Vec::new();

            let mut rollback: Vec<(PathBuf, Option<Vec<u8>>)> = Vec::new();
            let mut created_directories = Vec::new();

            let apply_result = (|| -> Result<(), WorkspaceError> {
                for relative in paths {
                    let relative = relative.trim_start_matches('/').to_owned();

                    if relative.is_empty() {
                        continue;
                    }

                    let target =
                        path::safe_path(&root, &relative, false).map_err(WorkspaceError::new)?;
                    let stats = this
                        .runtime
                        .block_on(session.filesystem.fs.stat(&relative))
                        .map_err(WorkspaceError::new)?;
                    if stats.as_ref().is_some_and(agentfs_sdk::Stats::is_directory) {
                        if target.exists() {
                            return Err(WorkspaceError::new(std::io::Error::new(
                                std::io::ErrorKind::AlreadyExists,
                                format!("{relative} appeared after approval"),
                            )));
                        }
                        std::fs::create_dir_all(&target).map_err(WorkspaceError::new)?;
                        created_directories.push(target);
                        changes.push(relative);
                        continue;
                    }

                    let Some(bytes) = this
                        .runtime
                        .block_on(session.filesystem.fs.read_file(&relative))
                        .map_err(WorkspaceError::new)?
                    else {
                        continue;
                    };

                    rollback.push((target.clone(), std::fs::read(&target).ok()));

                    if let Some(parent) = target.parent() {
                        std::fs::create_dir_all(parent).map_err(WorkspaceError::new)?;
                    }

                    write_atomic(&target, &bytes).map_err(WorkspaceError::new)?;

                    changes.push(relative);
                }
                Ok(())
            })();
            if let Err(error) = apply_result {
                for (target, previous) in rollback.into_iter().rev() {
                    match previous {
                        Some(bytes) => {
                            let _ = write_atomic(&target, &bytes);
                        }
                        None => {
                            let _ = std::fs::remove_file(&target);
                        }
                    }
                }
                for directory in created_directories.into_iter().rev() {
                    let _ = std::fs::remove_dir(&directory);
                }
                return Err(error);
            }

            this.sessions.write().remove(&root);

            changes.sort();
            Ok(changes)
        }))
    }

    fn discard_session(&self, root: PathBuf, session_id: String) -> WorkspaceFuture<()> {
        let this = self.clone();
        Box::pin(smol::unblock(move || {
            let root = path::canonical_root(&root).map_err(WorkspaceError::new)?;

            let session = this.sessions.read().get(&root).cloned().ok_or_else(|| {
                WorkspaceError::new(std::io::Error::other("no active AgentFS session"))
            })?;

            if session.id != session_id {
                return Err(WorkspaceError::new(std::io::Error::other(
                    "AgentFS session does not match",
                )));
            }

            this.sessions.write().remove(&root);
            let _audit_path = &session.database_path;
            Ok(())
        }))
    }
}

impl WorkspaceAccess for AgentFsWorkspace {
    fn prepare(
        &self,
        root: PathBuf,
        operation: WorkspaceOperation,
        allow_protected: bool,
    ) -> WorkspaceFuture<WorkspacePreview> {
        let Some(session) = self.session(&root) else {
            if operation.is_mutating() {
                return Box::pin(async {
                    Err(WorkspaceError::new(std::io::Error::other(
                        "AgentFS is unavailable; Work mode is read-only",
                    )))
                });
            }
            return self.fallback.prepare(root, operation, allow_protected);
        };

        if !matches!(
            operation,
            WorkspaceOperation::ReadFile { .. }
                | WorkspaceOperation::ApplyPatch { .. }
                | WorkspaceOperation::CreateFile { .. }
                | WorkspaceOperation::CreateDirectory { .. }
        ) {
            return self.fallback.prepare(root, operation, allow_protected);
        }

        let runtime = Arc::clone(&self.runtime);
        Box::pin(smol::unblock(move || {
            prepare_overlay(
                runtime.as_ref(),
                &session,
                &root,
                operation,
                allow_protected,
            )
            .map_err(WorkspaceError::new)
        }))
    }

    fn commit(&self, root: PathBuf, mutation: WorkspaceMutation) -> WorkspaceFuture<String> {
        let Some(session) = self.session(&root) else {
            return Box::pin(async {
                Err(WorkspaceError::new(std::io::Error::other(
                    "AgentFS is unavailable; Work mode is read-only",
                )))
            });
        };

        let runtime = Arc::clone(&self.runtime);
        Box::pin(smol::unblock(move || {
            commit_overlay(runtime.as_ref(), &session, &root, &mutation)
                .map_err(WorkspaceError::new)
        }))
    }
}

fn prepare_overlay(
    runtime: &tokio::runtime::Runtime,
    session: &Session,
    root: &Path,
    operation: WorkspaceOperation,
    allow_protected: bool,
) -> std::io::Result<WorkspacePreview> {
    let relative = operation.path().to_owned();

    let _ = path::safe_path(root, &relative, false)?;

    if path::protected(&relative) && !allow_protected {
        return Ok(WorkspacePreview {
            path: relative,
            summary: "protected file requires approval".into(),
            output: String::new(),
            diff: None,
            protected: true,
            mutation: None,
        });
    }

    match operation {
        WorkspaceOperation::ReadFile {
            start_line,
            line_count,
            ..
        } => {
            let bytes = overlay_read(runtime, session, &relative)?.ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::NotFound, "file does not exist")
            })?;

            let content = String::from_utf8(bytes)
                .map_err(|_| std::io::Error::other("read_file only supports UTF-8 text files"))?;

            let first = usize::try_from(start_line.unwrap_or(1).max(1) - 1).unwrap_or(0);

            let output = content
                .lines()
                .skip(first)
                .take(usize::try_from(line_count.unwrap_or(1000)).unwrap_or(1000))
                .collect::<Vec<_>>()
                .join("\n");

            Ok(WorkspacePreview {
                path: relative.clone(),
                summary: format!("read {relative}"),
                output,
                diff: None,
                protected: false,
                mutation: None,
            })
        }
        WorkspaceOperation::ApplyPatch { unified_diff, .. } => {
            let bytes = overlay_read(runtime, session, &relative)?.ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::NotFound, "file does not exist")
            })?;

            let source = String::from_utf8(bytes)
                .map_err(|_| std::io::Error::other("patch only supports UTF-8 text files"))?;

            let replacement = apply_unified(&source, &unified_diff)?;

            Ok(WorkspacePreview {
                path: relative.clone(),
                summary: format!("proposed patch for {relative}"),
                output: String::new(),
                diff: Some(display_diff(&relative, &source, &replacement)),
                protected: false,
                mutation: Some(WorkspaceMutation {
                    path: relative,
                    expected_digest: Some(digest(source.as_bytes())),
                    replacement: replacement.into_bytes(),
                    creates_file: false,
                    creates_directory: false,
                }),
            })
        }
        WorkspaceOperation::CreateFile { content, .. } => {
            if overlay_kind(runtime, session, &relative)?.is_some() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::AlreadyExists,
                    "file already exists",
                ));
            }

            Ok(WorkspacePreview {
                path: relative.clone(),
                summary: format!("proposed new file {relative}"),
                output: String::new(),
                diff: Some(display_diff(&relative, "", &content)),
                protected: false,
                mutation: Some(WorkspaceMutation {
                    path: relative,
                    expected_digest: None,
                    replacement: content.into_bytes(),
                    creates_file: true,
                    creates_directory: false,
                }),
            })
        }
        WorkspaceOperation::CreateDirectory { .. } => {
            prepare_overlay_create_directory(runtime, session, relative)
        }
        _ => unreachable!(),
    }
}

fn prepare_overlay_create_directory(
    runtime: &tokio::runtime::Runtime,
    session: &Session,
    relative: String,
) -> std::io::Result<WorkspacePreview> {
    if overlay_kind(runtime, session, &relative)?.is_some() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "path already exists",
        ));
    }

    Ok(WorkspacePreview {
        path: relative.clone(),
        summary: format!("proposed new directory {relative}"),
        output: String::new(),
        diff: None,
        protected: false,
        mutation: Some(WorkspaceMutation {
            path: relative,
            expected_digest: None,
            replacement: Vec::new(),
            creates_file: false,
            creates_directory: true,
        }),
    })
}

fn commit_overlay(
    runtime: &tokio::runtime::Runtime,
    session: &Session,
    root: &Path,
    mutation: &WorkspaceMutation,
) -> std::io::Result<String> {
    let target = path::safe_path(root, &mutation.path, false)?;
    if mutation.creates_directory {
        if overlay_kind(runtime, session, &mutation.path)?.is_some() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "path appeared after approval",
            ));
        }
        session
            .original_digests
            .lock()
            .entry(mutation.path.clone())
            .or_insert(None);
        overlay_mkdir(runtime, session, &mutation.path)?;
        return Ok(format!("staged directory {} in AgentFS", mutation.path));
    }
    let current = overlay_read(runtime, session, &mutation.path)?;
    let actual = current.as_deref().map(digest);

    if actual != mutation.expected_digest {
        return Err(std::io::Error::other(
            "file changed after the proposal was prepared",
        ));
    }

    let original = std::fs::read(&target).ok().map(|bytes| digest(&bytes));

    session
        .original_digests
        .lock()
        .entry(mutation.path.clone())
        .or_insert(original);

    overlay_write(runtime, session, &mutation.path, &mutation.replacement)?;
    Ok(format!("staged {} in AgentFS", mutation.path))
}

fn overlay_read(
    runtime: &tokio::runtime::Runtime,
    session: &Session,
    relative: &str,
) -> std::io::Result<Option<Vec<u8>>> {
    runtime.block_on(async {
        let mut inode = 1_i64;
        let mut stats = None;

        for component in Path::new(relative).components() {
            let name = component.as_os_str().to_string_lossy();
            let Some(found) = session
                .overlay
                .lookup(inode, &name)
                .await
                .map_err(io_other)?
            else {
                return Ok(None);
            };
            inode = found.ino;
            stats = Some(found);
        }

        let Some(stats) = stats else {
            return Ok(None);
        };

        let file = session
            .overlay
            .open(inode, libc::O_RDONLY)
            .await
            .map_err(io_other)?;

        let bytes = file
            .pread(0, u64::try_from(stats.size).unwrap_or(u64::MAX))
            .await
            .map_err(io_other)?;

        Ok(Some(bytes))
    })
}

fn overlay_kind(
    runtime: &tokio::runtime::Runtime,
    session: &Session,
    relative: &str,
) -> std::io::Result<Option<bool>> {
    runtime.block_on(async {
        let mut inode = 1_i64;
        let mut kind = None;
        for component in Path::new(relative).components() {
            let name = component.as_os_str().to_string_lossy();
            let Some(stats) = session
                .overlay
                .lookup(inode, &name)
                .await
                .map_err(io_other)?
            else {
                return Ok(None);
            };
            inode = stats.ino;
            kind = Some(stats.is_directory());
        }
        Ok(kind)
    })
}

fn overlay_mkdir(
    runtime: &tokio::runtime::Runtime,
    session: &Session,
    relative: &str,
) -> std::io::Result<()> {
    runtime.block_on(async {
        let mut inode = 1_i64;
        for component in Path::new(relative).components() {
            let name = component.as_os_str().to_string_lossy();
            inode = if let Some(stats) = session
                .overlay
                .lookup(inode, &name)
                .await
                .map_err(io_other)?
            {
                if !stats.is_directory() {
                    return Err(std::io::Error::other("directory parent is not a directory"));
                }
                stats.ino
            } else {
                session
                    .overlay
                    .mkdir(inode, &name, 0o755, 0, 0)
                    .await
                    .map_err(io_other)?
                    .ino
            };
        }
        Ok(())
    })
}

fn overlay_write(
    runtime: &tokio::runtime::Runtime,
    session: &Session,
    relative: &str,
    bytes: &[u8],
) -> std::io::Result<()> {
    runtime.block_on(async {
        let components = Path::new(relative)
            .components()
            .map(|part| part.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>();

        let (name, parents) = components
            .split_last()
            .ok_or_else(|| std::io::Error::other("empty overlay path"))?;

        let mut parent_inode = 1_i64;

        for parent in parents {
            parent_inode = if let Some(stats) = session
                .overlay
                .lookup(parent_inode, parent)
                .await
                .map_err(io_other)?
            {
                stats.ino
            } else {
                session
                    .overlay
                    .mkdir(parent_inode, parent, 0o755, 0, 0)
                    .await
                    .map_err(io_other)?
                    .ino
            };
        }

        let file = if let Some(stats) = session
            .overlay
            .lookup(parent_inode, name)
            .await
            .map_err(io_other)?
        {
            session
                .overlay
                .open(stats.ino, libc::O_RDWR)
                .await
                .map_err(io_other)?
        } else {
            session
                .overlay
                .create_file(parent_inode, name, 0o644, 0, 0)
                .await
                .map_err(io_other)?
                .1
        };

        file.truncate(0).await.map_err(io_other)?;
        file.pwrite(0, bytes).await.map_err(io_other)?;
        file.fsync().await.map_err(io_other)
    })
}

fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| std::io::Error::other("destination has no parent"))?;

    let temporary = tempfile::NamedTempFile::new_in(parent)?;
    std::fs::write(temporary.path(), bytes)?;
    temporary.as_file().sync_all()?;
    std::fs::rename(temporary.path(), path)
}

fn io_other(error: impl std::fmt::Display) -> std::io::Error {
    std::io::Error::other(error.to_string())
}

#[cfg(test)]
#[path = "../test/agentfs.rs"]
mod tests;
