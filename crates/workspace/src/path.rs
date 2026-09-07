use std::{
    io,
    path::{Component, Path, PathBuf},
};

pub const MAX_FILE_BYTES: u64 = 1024 * 1024;
pub const MAX_FILE_BYTES_USIZE: usize = 1024 * 1024;
pub const MAX_SEARCH_RESULTS: usize = 100;
pub const MAX_SEARCH_OUTPUT_BYTES: usize = 128 * 1024;
pub const MAX_LIST_ENTRIES: usize = 500;

pub fn canonical_root(root: &Path) -> io::Result<PathBuf> {
    let root = root.canonicalize()?;
    if !root.is_dir() {
        return Err(invalid("workspace root is not a directory"));
    }
    Ok(root)
}

pub fn safe_path(root: &Path, relative: &str, must_exist: bool) -> io::Result<PathBuf> {
    let relative_path = Path::new(relative);
    if relative.trim().is_empty() || relative_path.is_absolute() {
        return Err(invalid("workspace paths must be non-empty and relative"));
    }
    if relative_path.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        return Err(invalid(
            "workspace paths cannot escape through parent components",
        ));
    }
    if relative_path.components().any(|component| {
        component
            .as_os_str()
            .to_string_lossy()
            .eq_ignore_ascii_case(".git")
    }) {
        return Err(invalid("the .git directory is not available to the agent"));
    }

    let candidate = root.join(relative_path);
    if candidate.exists() {
        let canonical = candidate.canonicalize()?;
        if !canonical.starts_with(root) {
            return Err(invalid("the resolved path is outside the workspace"));
        }
        return Ok(canonical);
    }
    if must_exist {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "workspace path does not exist",
        ));
    }

    let mut parent = candidate
        .parent()
        .ok_or_else(|| invalid("workspace path has no parent"))?;
    while !parent.exists() {
        parent = parent
            .parent()
            .ok_or_else(|| invalid("workspace path has no existing parent"))?;
    }
    let parent = parent.canonicalize()?;
    if !parent.starts_with(root) {
        return Err(invalid("the destination parent is outside the workspace"));
    }
    Ok(candidate)
}

pub fn protected(relative: &str) -> bool {
    let path = Path::new(relative);
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    name == ".env"
        || name.starts_with(".env.")
        || matches!(name, ".npmrc" | ".pypirc" | ".netrc")
        || matches!(name, "id_rsa" | "id_dsa" | "id_ecdsa" | "id_ed25519")
        || matches!(
            path.extension().and_then(|value| value.to_str()),
            Some("pem" | "key" | "p12" | "pfx")
        )
}

pub fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}
