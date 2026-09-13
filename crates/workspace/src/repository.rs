use std::{
    fs, io,
    path::{Component, Path, PathBuf},
    process::{Command, Output},
};

use magenta_core::{
    RepositoryAccess, RepositoryChange, RepositoryChangeKind, RepositoryCommit, RepositoryDiff,
    RepositoryDiffArea, RepositoryError, RepositoryErrorKind, RepositoryFuture, RepositoryStatus,
};

const MAX_DIFF_FILE_BYTES: u64 = 1024 * 1024;

#[derive(Clone, Copy, Debug, Default)]
pub struct LocalRepository;

impl RepositoryAccess for LocalRepository {
    fn status(&self, root: PathBuf) -> RepositoryFuture<RepositoryStatus> {
        Box::pin(smol::unblock(move || status(&root)))
    }

    fn diff(
        &self,
        root: PathBuf,
        path: String,
        area: RepositoryDiffArea,
    ) -> RepositoryFuture<RepositoryDiff> {
        Box::pin(smol::unblock(move || diff(&root, &path, area)))
    }

    fn stage(&self, root: PathBuf, paths: Vec<String>) -> RepositoryFuture<()> {
        Box::pin(smol::unblock(move || mutate_paths(&root, "add", &paths)))
    }

    fn unstage(&self, root: PathBuf, paths: Vec<String>) -> RepositoryFuture<()> {
        Box::pin(smol::unblock(move || unstage(&root, &paths)))
    }

    fn commit(&self, root: PathBuf, message: String) -> RepositoryFuture<RepositoryCommit> {
        Box::pin(smol::unblock(move || commit(&root, &message)))
    }
}

fn status(root: &Path) -> Result<RepositoryStatus, RepositoryError> {
    repository_root(root)?;

    let output = git(
        root,
        &[
            "status",
            "--porcelain=v1",
            "-z",
            "--branch",
            "--untracked-files=all",
        ],
    )?;
    success(output).and_then(|output| parse_status(&output.stdout))
}

fn parse_status(bytes: &[u8]) -> Result<RepositoryStatus, RepositoryError> {
    let records = bytes
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty())
        .collect::<Vec<_>>();
    let mut branch = None;
    let mut detached = false;
    let mut unborn = false;
    let mut changes = Vec::new();
    let mut index = 0;

    while index < records.len() {
        let record = String::from_utf8_lossy(records[index]);

        if let Some(header) = record.strip_prefix("## ") {
            unborn = header.contains("No commits yet") || header.contains("Initial commit");
            detached = header.starts_with("HEAD ") || header == "HEAD (no branch)";
            branch = (!detached).then(|| {
                header
                    .split("...")
                    .next()
                    .unwrap_or(header)
                    .trim()
                    .to_owned()
            });
            index += 1;
            continue;
        }

        if record.len() < 4 {
            return Err(error(
                RepositoryErrorKind::Other,
                "invalid Git status response",
            ));
        }
        let mut chars = record.chars();
        let x = chars.next().unwrap_or(' ');
        let y = chars.next().unwrap_or(' ');
        let path = record[3..].to_owned();
        let conflicted = is_conflicted(x, y);

        let rename = matches!(x, 'R' | 'C') || matches!(y, 'R' | 'C');
        let original_path = if rename {
            index += 1;
            records
                .get(index)
                .map(|value| String::from_utf8_lossy(value).into_owned())
        } else {
            None
        };

        changes.push(RepositoryChange {
            path,
            original_path,
            staged: if conflicted {
                None
            } else {
                status_kind(x, false)
            },
            unstaged: if conflicted {
                Some(RepositoryChangeKind::Conflicted)
            } else {
                status_kind(y, x == '?' && y == '?')
            },
        });
        index += 1;
    }

    changes.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(RepositoryStatus {
        branch,
        detached,
        unborn,
        changes,
    })
}

const fn is_conflicted(index: char, worktree: char) -> bool {
    matches!(
        (index, worktree),
        ('D' | 'U', 'D') | ('A' | 'D' | 'U', 'U') | ('A' | 'U', 'A')
    )
}

const fn status_kind(code: char, untracked: bool) -> Option<RepositoryChangeKind> {
    if untracked {
        return Some(RepositoryChangeKind::Untracked);
    }
    match code {
        'A' => Some(RepositoryChangeKind::Added),
        'M' => Some(RepositoryChangeKind::Modified),
        'D' => Some(RepositoryChangeKind::Deleted),
        'R' | 'C' => Some(RepositoryChangeKind::Renamed),
        'U' => Some(RepositoryChangeKind::Conflicted),
        _ => None,
    }
}

fn diff(
    root: &Path,
    path: &str,
    area: RepositoryDiffArea,
) -> Result<RepositoryDiff, RepositoryError> {
    repository_root(root)?;
    validate_path(path)?;

    let state = status(root)?
        .changes
        .into_iter()
        .find(|change| change.path == path);
    if area == RepositoryDiffArea::Unstaged
        && state.as_ref().and_then(|change| change.unstaged)
            == Some(RepositoryChangeKind::Untracked)
    {
        return untracked_diff(root, path);
    }

    let mut args = vec!["diff", "--no-ext-diff", "--no-textconv", "--no-color"];
    if area == RepositoryDiffArea::Staged {
        args.push("--cached");
    }

    args.extend(["--", path]);
    let output = success(git(root, &args)?)?;
    let text = String::from_utf8_lossy(&output.stdout).into_owned();
    let binary = text.contains("Binary files ") || text.contains("GIT binary patch");

    Ok(RepositoryDiff {
        path: path.to_owned(),
        area,
        unified_diff: text,
        binary,
    })
}

fn untracked_diff(root: &Path, path: &str) -> Result<RepositoryDiff, RepositoryError> {
    let file = root.join(path);
    let metadata = fs::metadata(&file).map_err(other)?;

    if metadata.len() > MAX_DIFF_FILE_BYTES {
        return Ok(RepositoryDiff {
            path: path.to_owned(),
            area: RepositoryDiffArea::Unstaged,
            unified_diff: String::new(),
            binary: true,
        });
    }

    let bytes = fs::read(&file).map_err(other)?;

    if bytes.contains(&0) {
        return Ok(RepositoryDiff {
            path: path.to_owned(),
            area: RepositoryDiffArea::Unstaged,
            unified_diff: String::new(),
            binary: true,
        });
    }

    let content = String::from_utf8(bytes).map_err(other)?;
    let mut unified = format!(
        "--- /dev/null\n+++ b/{path}\n@@ -0,0 +1,{} @@\n",
        content.lines().count()
    );
    for line in content.split_inclusive('\n') {
        unified.push('+');
        unified.push_str(line);
    }

    if !content.is_empty() && !content.ends_with('\n') {
        unified.push_str("\n\\ No newline at end of file\n");
    }

    Ok(RepositoryDiff {
        path: path.to_owned(),
        area: RepositoryDiffArea::Unstaged,
        unified_diff: unified,
        binary: false,
    })
}

fn mutate_paths(root: &Path, operation: &str, paths: &[String]) -> Result<(), RepositoryError> {
    repository_root(root)?;
    validate_paths(paths)?;

    if paths.is_empty() {
        return Ok(());
    }

    let mut args = vec![operation, "--"];
    args.extend(paths.iter().map(String::as_str));
    success(git(root, &args)?)?;

    Ok(())
}

fn unstage(root: &Path, paths: &[String]) -> Result<(), RepositoryError> {
    repository_root(root)?;
    validate_paths(paths)?;

    if paths.is_empty() {
        return Ok(());
    }

    let mut args = vec!["restore", "--staged", "--"];
    args.extend(paths.iter().map(String::as_str));
    let output = git(root, &args)?;
    if output.status.success() {
        return Ok(());
    }

    if status(root)?.unborn {
        let mut fallback = vec!["rm", "--cached", "--quiet", "--"];
        fallback.extend(paths.iter().map(String::as_str));
        success(git(root, &fallback)?)?;
        return Ok(());
    }

    Err(command_error(&output))
}

fn commit(root: &Path, message: &str) -> Result<RepositoryCommit, RepositoryError> {
    repository_root(root)?;
    let message = message.trim();

    if message.is_empty() {
        return Err(error(
            RepositoryErrorKind::Conflict,
            "commit message cannot be empty",
        ));
    }
    success(git(
        root,
        &["-c", "commit.gpgsign=false", "commit", "-m", message],
    )?)?;
    let oid = success(git(root, &["rev-parse", "--short=12", "HEAD"])?)?;
    let summary = success(git(root, &["log", "-1", "--pretty=%s"])?)?;

    Ok(RepositoryCommit {
        oid: String::from_utf8_lossy(&oid.stdout).trim().to_owned(),
        summary: String::from_utf8_lossy(&summary.stdout).trim().to_owned(),
    })
}

fn repository_root(root: &Path) -> Result<PathBuf, RepositoryError> {
    let selected = root.canonicalize().map_err(other)?;
    let output = git(&selected, &["rev-parse", "--show-toplevel"])?;

    if !output.status.success() {
        return Err(command_error_kind(
            &output,
            RepositoryErrorKind::NotRepository,
        ));
    }

    let discovered = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim().to_string())
        .canonicalize()
        .map_err(other)?;

    if discovered != selected {
        return Err(error(
            RepositoryErrorKind::WorkspaceIsNotRepositoryRoot,
            "the selected workspace is inside a repository; open the repository root as the project",
        ));
    }

    Ok(discovered)
}

fn validate_paths(paths: &[String]) -> Result<(), RepositoryError> {
    paths.iter().try_for_each(|path| validate_path(path))
}

fn validate_path(path: &str) -> Result<(), RepositoryError> {
    let candidate = Path::new(path);
    if path.is_empty()
        || candidate.is_absolute()
        || candidate.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
        || candidate
            .components()
            .any(|component| component.as_os_str() == ".git")
    {
        return Err(error(
            RepositoryErrorKind::InvalidPath,
            "repository paths must be relative and cannot access .git",
        ));
    }
    Ok(())
}

fn git(root: &Path, args: &[&str]) -> Result<Output, RepositoryError> {
    Command::new("git")
        .arg("--no-pager")
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_EDITOR", "true")
        .output()
        .map_err(|source| {
            RepositoryError::new(
                if source.kind() == io::ErrorKind::NotFound {
                    RepositoryErrorKind::GitUnavailable
                } else {
                    RepositoryErrorKind::Other
                },
                source,
            )
        })
}

fn success(output: Output) -> Result<Output, RepositoryError> {
    if output.status.success() {
        Ok(output)
    } else {
        Err(command_error(&output))
    }
}

fn command_error(output: &Output) -> RepositoryError {
    command_error_kind(output, RepositoryErrorKind::Other)
}

fn command_error_kind(output: &Output, kind: RepositoryErrorKind) -> RepositoryError {
    let detail = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    error(
        kind,
        if detail.is_empty() {
            "Git command failed"
        } else {
            &detail
        },
    )
}

fn other(source: impl std::error::Error + Send + Sync + 'static) -> RepositoryError {
    RepositoryError::new(RepositoryErrorKind::Other, source)
}

fn error(kind: RepositoryErrorKind, message: &str) -> RepositoryError {
    RepositoryError::new(kind, io::Error::other(message.to_owned()))
}

#[cfg(test)]
#[path = "../test/repository.rs"]
mod tests;
