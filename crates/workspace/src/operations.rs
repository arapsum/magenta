use std::{
    fs, io,
    path::{Path, PathBuf},
};

use globset::{Glob, GlobSetBuilder};
use ignore::WalkBuilder;
use magenta_core::{
    WorkspaceAccess, WorkspaceError, WorkspaceFuture, WorkspaceMutation, WorkspaceOperation,
    WorkspacePreview,
};
use sha2::{Digest, Sha256};

use crate::{patch::apply_unified, path};

const MAX_PATCH_BYTES: usize = 256 * 1024;

#[derive(Clone, Copy, Debug, Default)]
pub struct LocalWorkspace;

impl WorkspaceAccess for LocalWorkspace {
    fn prepare(
        &self,
        root: PathBuf,
        operation: WorkspaceOperation,
        allow_protected: bool,
    ) -> WorkspaceFuture<WorkspacePreview> {
        Box::pin(smol::unblock(move || {
            prepare(&root, &operation, allow_protected).map_err(WorkspaceError::new)
        }))
    }

    fn commit(&self, root: PathBuf, mutation: WorkspaceMutation) -> WorkspaceFuture<String> {
        Box::pin(smol::unblock(move || {
            commit(&root, &mutation).map_err(WorkspaceError::new)
        }))
    }
}

fn prepare(
    root: &Path,
    operation: &WorkspaceOperation,
    allow_protected: bool,
) -> io::Result<WorkspacePreview> {
    let root = path::canonical_root(root)?;
    match operation {
        WorkspaceOperation::ListFiles {
            path: relative,
            depth,
        } => list_files(&root, relative, *depth),
        WorkspaceOperation::SearchText {
            query,
            path: relative,
            glob,
        } => search_text(&root, query, relative, glob.as_deref()),
        WorkspaceOperation::ReadFile {
            path: relative,
            start_line,
            line_count,
        } => read_file(&root, relative, *start_line, *line_count, allow_protected),
        WorkspaceOperation::ApplyPatch {
            path: relative,
            unified_diff,
        } => apply_patch(&root, relative, unified_diff),
        WorkspaceOperation::CreateFile {
            path: relative,
            content,
        } => create_file(&root, relative, content),
    }
}

fn list_files(root: &Path, relative: &str, depth: u8) -> io::Result<WorkspacePreview> {
    let directory = path::safe_path(root, relative, true)?;
    if !directory.is_dir() {
        return Err(invalid("list_files requires a directory"));
    }
    let mut entries = Vec::new();
    let walker = WalkBuilder::new(&directory)
        .hidden(false)
        .standard_filters(true)
        .max_depth(Some(usize::from(depth) + 1))
        .build();
    for entry in walker {
        let entry = entry.map_err(|error| invalid(&error.to_string()))?;
        if entry.depth() == 0
            || entry
                .path()
                .components()
                .any(|component| component.as_os_str() == ".git")
        {
            continue;
        }
        let suffix = if entry.file_type().is_some_and(|kind| kind.is_dir()) {
            "/"
        } else {
            ""
        };
        entries.push(format!("{}{suffix}", path::relative(root, entry.path())));
        if entries.len() >= path::MAX_LIST_ENTRIES {
            break;
        }
    }
    entries.sort_unstable();
    Ok(WorkspacePreview {
        path: relative.to_owned(),
        summary: format!("{} workspace entries", entries.len()),
        output: entries.join("\n"),
        diff: None,
        protected: false,
        mutation: None,
    })
}

fn search_text(
    root: &Path,
    query: &str,
    relative: &str,
    glob: Option<&str>,
) -> io::Result<WorkspacePreview> {
    if query.is_empty() {
        return Err(invalid("search_text requires a non-empty query"));
    }
    let directory = path::safe_path(root, relative, true)?;
    let matcher = glob
        .map(|value| {
            let mut builder = GlobSetBuilder::new();
            builder.add(Glob::new(value).map_err(|error| invalid(&error.to_string()))?);
            builder.build().map_err(|error| invalid(&error.to_string()))
        })
        .transpose()?;
    let mut results = Vec::new();
    let walker = WalkBuilder::new(directory)
        .hidden(false)
        .standard_filters(true)
        .build();
    for entry in walker {
        let entry = entry.map_err(|error| invalid(&error.to_string()))?;
        let Some(kind) = entry.file_type() else {
            continue;
        };
        if !kind.is_file()
            || entry
                .path()
                .components()
                .any(|component| component.as_os_str() == ".git")
        {
            continue;
        }
        let relative_path = path::relative(root, entry.path());
        if matcher
            .as_ref()
            .is_some_and(|set| !set.is_match(&relative_path))
            || path::protected(&relative_path)
        {
            continue;
        }
        let metadata = fs::metadata(entry.path())?;
        if metadata.len() > path::MAX_FILE_BYTES {
            continue;
        }
        let content = fs::read_to_string(entry.path())
            .map_err(|_| invalid("search only supports UTF-8 text files"))?;
        for (line_number, line) in content.lines().enumerate() {
            if line.contains(query) {
                results.push(format!(
                    "{relative_path}:{}:{}",
                    line_number + 1,
                    line.trim()
                ));
                if results.len() >= path::MAX_SEARCH_RESULTS
                    || results.join("\n").len() >= path::MAX_SEARCH_OUTPUT_BYTES
                {
                    break;
                }
            }
        }
        if results.len() >= path::MAX_SEARCH_RESULTS
            || results.join("\n").len() >= path::MAX_SEARCH_OUTPUT_BYTES
        {
            break;
        }
    }
    Ok(WorkspacePreview {
        path: relative.to_owned(),
        summary: format!("{} search matches", results.len()),
        output: results.join("\n"),
        diff: None,
        protected: false,
        mutation: None,
    })
}

fn read_file(
    root: &Path,
    relative: &str,
    start_line: Option<u32>,
    line_count: Option<u32>,
    allow_protected: bool,
) -> io::Result<WorkspacePreview> {
    let file = path::safe_path(root, relative, true)?;
    if !file.is_file() {
        return Err(invalid("read_file requires a regular file"));
    }
    let protected = path::protected(relative);
    if protected && !allow_protected {
        return Ok(WorkspacePreview {
            path: relative.to_owned(),
            summary: "protected file requires approval".to_owned(),
            output: String::new(),
            diff: None,
            protected: true,
            mutation: None,
        });
    }
    let metadata = fs::metadata(&file)?;
    if metadata.len() > path::MAX_FILE_BYTES {
        return Err(invalid("file exceeds the one MiB read limit"));
    }
    let content = fs::read_to_string(&file)
        .map_err(|_| invalid("read_file only supports UTF-8 text files"))?;
    let first = usize::try_from(start_line.unwrap_or(1).max(1) - 1)
        .map_err(|_| invalid("invalid start line"))?;
    let count =
        usize::try_from(line_count.unwrap_or(1_000)).map_err(|_| invalid("invalid line count"))?;
    let output = content
        .lines()
        .skip(first)
        .take(count)
        .collect::<Vec<_>>()
        .join("\n");
    if output.len() > 256 * 1024 {
        return Err(invalid(
            "requested file range exceeds the 256 KiB response limit",
        ));
    }
    Ok(WorkspacePreview {
        path: relative.to_owned(),
        summary: format!("read {relative}"),
        output,
        diff: None,
        protected: false,
        mutation: None,
    })
}

fn apply_patch(root: &Path, relative: &str, unified_diff: &str) -> io::Result<WorkspacePreview> {
    if unified_diff.len() > MAX_PATCH_BYTES {
        return Err(invalid("patch exceeds the 256 KiB limit"));
    }
    let file = path::safe_path(root, relative, true)?;
    if !file.is_file() || path::protected(relative) {
        return Err(invalid("patching this path is not allowed"));
    }
    let source =
        fs::read_to_string(&file).map_err(|_| invalid("patch only supports UTF-8 text files"))?;
    let replacement = apply_unified(&source, unified_diff)?;
    if replacement.len() > path::MAX_FILE_BYTES_USIZE {
        return Err(invalid("patched file exceeds the one MiB limit"));
    }
    let digest = digest(source.as_bytes());
    Ok(WorkspacePreview {
        path: relative.to_owned(),
        summary: format!("proposed patch for {relative}"),
        output: String::new(),
        diff: Some(display_diff(relative, &source, &replacement)),
        protected: false,
        mutation: Some(WorkspaceMutation {
            path: relative.to_owned(),
            expected_digest: Some(digest),
            replacement: replacement.into_bytes(),
            creates_file: false,
        }),
    })
}

fn create_file(root: &Path, relative: &str, content: &str) -> io::Result<WorkspacePreview> {
    if path::protected(relative) || content.len() > path::MAX_FILE_BYTES_USIZE {
        return Err(invalid(
            "creating this file is not allowed or exceeds the one MiB limit",
        ));
    }
    let file = path::safe_path(root, relative, false)?;
    if file.exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "file already exists",
        ));
    }
    Ok(WorkspacePreview {
        path: relative.to_owned(),
        summary: format!("proposed new file {relative}"),
        output: String::new(),
        diff: Some(display_diff(relative, "", content)),
        protected: false,
        mutation: Some(WorkspaceMutation {
            path: relative.to_owned(),
            expected_digest: None,
            replacement: content.as_bytes().to_vec(),
            creates_file: true,
        }),
    })
}

fn commit(root: &Path, mutation: &WorkspaceMutation) -> io::Result<String> {
    let root = path::canonical_root(root)?;
    let mut file = path::safe_path(&root, &mutation.path, !mutation.creates_file)?;
    if mutation.creates_file {
        let parent = file
            .parent()
            .ok_or_else(|| invalid("destination has no parent"))?;
        fs::create_dir_all(parent)?;
        file = path::safe_path(&root, &mutation.path, false)?;
        if file.exists() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "file appeared after approval",
            ));
        }
    } else {
        let current = fs::read(&file)?;
        if mutation.expected_digest.as_deref() != Some(digest(&current).as_str()) {
            return Err(invalid("file changed after the proposal was prepared"));
        }
    }
    let parent = file
        .parent()
        .ok_or_else(|| invalid("destination has no parent"))?;
    let temporary = tempfile::NamedTempFile::new_in(parent)?;
    fs::write(temporary.path(), &mutation.replacement)?;
    temporary.as_file().sync_all()?;
    fs::rename(temporary.path(), &file)?;
    let action = if mutation.creates_file {
        "created"
    } else {
        "updated"
    };
    Ok(format!("{action} {}", mutation.path))
}

fn display_diff(path: &str, before: &str, after: &str) -> String {
    let mut diff = format!("--- a/{path}\n+++ b/{path}\n@@\n");
    for line in before.lines() {
        diff.push('-');
        diff.push_str(line);
        diff.push('\n');
    }
    for line in after.lines() {
        diff.push('+');
        diff.push_str(line);
        diff.push('\n');
    }
    diff
}

fn digest(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use magenta_core::WorkspaceOperation;

    #[test]
    fn creating_a_file_can_materialize_missing_parent_directories() {
        let directory = tempfile::tempdir().expect("workspace should exist");
        let operation = WorkspaceOperation::CreateFile {
            path: "hello-gtk/main.c".to_owned(),
            content: "int main(void) { return 0; }\n".to_owned(),
        };
        let preview = prepare(directory.path(), &operation, false).expect("preview should work");
        let mutation = preview.mutation.expect("create should produce a mutation");

        assert_eq!(
            commit(directory.path(), &mutation).expect("file should be created"),
            "created hello-gtk/main.c"
        );
        assert_eq!(
            fs::read_to_string(directory.path().join("hello-gtk/main.c"))
                .expect("created file should be readable"),
            "int main(void) { return 0; }\n"
        );
    }
}
