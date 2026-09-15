use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use ignore::WalkBuilder;
use magenta_core::{
    AgentDataFuture, CodeChunk, CodeIndex, CodeIndexMaintainer, CodeIndexReport, EmbeddingProvider,
    StorageError, StorageErrorKind,
};
use sha2::{Digest as _, Sha256};

const MAX_SOURCE_BYTES: u64 = 1024 * 1024;
const CHUNK_LINES: usize = 120;
const CHUNK_OVERLAP: usize = 20;

#[derive(Clone)]
pub struct WorkspaceCodeIndexer {
    index: Arc<dyn CodeIndex>,
    embeddings: Arc<dyn EmbeddingProvider>,
}

impl WorkspaceCodeIndexer {
    #[must_use]
    pub fn new(index: Arc<dyn CodeIndex>, embeddings: Arc<dyn EmbeddingProvider>) -> Self {
        Self { index, embeddings }
    }
}

impl CodeIndexMaintainer for WorkspaceCodeIndexer {
    fn refresh(&self, project_root: PathBuf) -> AgentDataFuture<CodeIndexReport> {
        let index = Arc::clone(&self.index);
        let embeddings = Arc::clone(&self.embeddings);

        Box::pin(async move {
            let scan_root = project_root.clone();
            let files = smol::unblock(move || scan(&scan_root)).await?;

            let mut report = CodeIndexReport::default();

            for file in files {
                if index
                    .file_hash(project_root.clone(), file.path.clone())
                    .await?
                    == Some(file.hash.clone())
                {
                    report.unchanged_files += 1;
                    continue;
                }

                let mut chunks = chunk_file(&file);

                if let Ok(vectors) = embeddings
                    .embed(chunks.iter().map(|chunk| chunk.content.clone()).collect())
                    .await
                {
                    for (chunk, vector) in chunks.iter_mut().zip(vectors) {
                        chunk.embedding = Some(vector);
                    }
                }

                index
                    .replace_file(project_root.clone(), file.path, file.hash, chunks)
                    .await?;
                report.indexed_files += 1;
            }

            Ok(report)
        })
    }
}

struct SourceFile {
    path: String,
    language: String,
    hash: String,
    content: String,
}

fn scan(root: &Path) -> Result<Vec<SourceFile>, StorageError> {
    if !root.is_dir() {
        return Err(failure("workspace is unavailable"));
    }

    let mut result = Vec::new();

    for entry in WalkBuilder::new(root)
        .hidden(false)
        .git_ignore(true)
        .git_exclude(true)
        .build()
    {
        let entry =
            entry.map_err(|error| StorageError::new(StorageErrorKind::Unavailable, error))?;
        let Some(kind) = entry.file_type() else {
            continue;
        };

        if !kind.is_file() {
            continue;
        }

        let path = entry.path();
        let Some(language) = language(path) else {
            continue;
        };

        let metadata = entry
            .metadata()
            .map_err(|error| StorageError::new(StorageErrorKind::Unavailable, error))?;

        if metadata.len() > MAX_SOURCE_BYTES {
            continue;
        }

        let content = std::fs::read_to_string(path)
            .map_err(|error| StorageError::new(StorageErrorKind::Unavailable, error))?;

        let relative = path
            .strip_prefix(root)
            .map_err(|_| failure("indexed path escaped workspace"))?
            .to_string_lossy()
            .replace('\\', "/");

        let hash = format!("{:x}", Sha256::digest(content.as_bytes()));

        result.push(SourceFile {
            path: relative,
            language: language.to_owned(),
            hash,
            content,
        });
    }

    Ok(result)
}

fn chunk_file(file: &SourceFile) -> Vec<CodeChunk> {
    let lines = file.content.lines().collect::<Vec<_>>();

    if lines.is_empty() {
        return Vec::new();
    }

    let mut chunks = Vec::new();
    let mut start = 0;

    while start < lines.len() {
        let end = (start + CHUNK_LINES).min(lines.len());

        let content = lines[start..end].join("\n");
        let symbol = lines[start..end]
            .iter()
            .find_map(|line| symbol_name(line, &file.language));

        chunks.push(CodeChunk {
            path: file.path.clone(),
            language: file.language.clone(),
            symbol,
            start_line: u32::try_from(start + 1).unwrap_or(u32::MAX),
            end_line: u32::try_from(end).unwrap_or(u32::MAX),
            content,
            content_hash: file.hash.clone(),
            embedding: None,
            session_id: None,
        });

        if end == lines.len() {
            break;
        }

        start = end.saturating_sub(CHUNK_OVERLAP);
    }
    chunks
}

fn language(path: &Path) -> Option<&'static str> {
    match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "rs" => Some("rust"),
        "js" | "jsx" => Some("javascript"),
        "ts" | "tsx" => Some("typescript"),
        "py" => Some("python"),
        _ => None,
    }
}

fn symbol_name(line: &str, language: &str) -> Option<String> {
    let line = line.trim_start();
    let markers: &[&str] = match language {
        "rust" => &["fn ", "struct ", "enum ", "trait ", "impl "],
        "python" => &["def ", "class "],
        _ => &[
            "function ",
            "class ",
            "const ",
            "export function ",
            "export class ",
        ],
    };

    markers
        .iter()
        .find_map(|marker| line.strip_prefix(marker))
        .and_then(|tail| {
            let name = tail
                .split(|character: char| !(character.is_alphanumeric() || character == '_'))
                .next()
                .unwrap_or_default();
            (!name.is_empty()).then(|| name.to_owned())
        })
}

fn failure(message: &'static str) -> StorageError {
    StorageError::new(
        StorageErrorKind::Unavailable,
        std::io::Error::other(message),
    )
}
