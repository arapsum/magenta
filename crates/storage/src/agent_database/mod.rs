//! One local Turso database per project for agent-owned durable state.

use std::{
    cmp::Ordering,
    path::{Path, PathBuf},
    sync::Arc,
};

use magenta_core::{
    AgentContentCache, AgentDataFuture, AgentMemory, AgentMemoryStore, AgentSession,
    AgentSessionState, AgentSessionStore, CachedContent, CodeChunk, CodeIndex, CodeMatch,
    ConversationId, MemoryKind, MemoryMatch, MemoryState, NewAgentMemory, StorageError,
    StorageErrorKind, Timestamp,
};
use sha2::{Digest as _, Sha256};
use turso::{Connection, Row, params};

mod cache;
mod index;
mod memory;
mod sessions;

const SCHEMA_VERSION: i64 = 1;
const MAX_CACHE_BYTES: i64 = 256 * 1024 * 1024;
const MAX_CACHE_VERSIONS_PER_PATH: i64 = 3;

const SCHEMA: &str = r"
CREATE TABLE IF NOT EXISTS _magenta_schema (
    component TEXT PRIMARY KEY,
    version INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS memories (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    kind TEXT NOT NULL,
    state TEXT NOT NULL,
    content TEXT NOT NULL,
    normalized_content TEXT NOT NULL,
    source_conversation_id INTEGER,
    confidence REAL NOT NULL,
    embedding BLOB,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS memory_state_recency ON memories(state, updated_at DESC);
CREATE TABLE IF NOT EXISTS indexed_files (
    path TEXT PRIMARY KEY,
    content_hash TEXT NOT NULL,
    indexed_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS code_chunks (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    path TEXT NOT NULL,
    language TEXT NOT NULL,
    symbol TEXT,
    start_line INTEGER NOT NULL,
    end_line INTEGER NOT NULL,
    content TEXT NOT NULL,
    normalized_content TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    embedding BLOB,
    session_id TEXT
);
CREATE INDEX IF NOT EXISTS code_chunk_path ON code_chunks(path, session_id);
CREATE TABLE IF NOT EXISTS content_cache (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    path TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    version INTEGER NOT NULL,
    content TEXT NOT NULL,
    compact_context TEXT NOT NULL,
    byte_size INTEGER NOT NULL,
    accessed_at INTEGER NOT NULL,
    UNIQUE(path, content_hash)
);
CREATE INDEX IF NOT EXISTS content_cache_lru ON content_cache(accessed_at);
CREATE TABLE IF NOT EXISTS agent_sessions (
    id TEXT PRIMARY KEY,
    conversation_id INTEGER NOT NULL,
    agentfs_path BLOB NOT NULL,
    state TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS session_checkpoints (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT NOT NULL,
    kind TEXT NOT NULL,
    payload BLOB NOT NULL,
    created_at INTEGER NOT NULL
);
";

#[derive(Clone)]
pub struct TursoAgentDatabase {
    projects_directory: Arc<PathBuf>,
}

impl TursoAgentDatabase {
    #[must_use]
    pub fn new(projects_directory: PathBuf) -> Self {
        Self {
            projects_directory: Arc::new(projects_directory),
        }
    }

    #[must_use]
    pub fn database_path(&self, project_root: &Path) -> PathBuf {
        let mut digest = Sha256::new();
        digest.update(super::records::encode_path(project_root));

        let key = format!("{:x}", digest.finalize());
        self.projects_directory.join(key).join("agent.db")
    }

    async fn connect(&self, project_root: &Path) -> Result<Connection, StorageError> {
        let path = self.database_path(project_root);

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(super::unavailable)?;
        }

        let path = path.to_string_lossy().into_owned();

        let database = turso::Builder::new_local(&path)
            .build()
            .await
            .map_err(database_error)?;

        let connection = database.connect().map_err(database_error)?;

        connection
            .execute("PRAGMA foreign_keys = ON", ())
            .await
            .map_err(database_error)?;

        connection
            .execute_batch(SCHEMA)
            .await
            .map_err(database_error)?;

        connection
            .execute(
                "INSERT INTO _magenta_schema(component, version) VALUES ('agent', ?1) \
             ON CONFLICT(component) DO UPDATE SET version = excluded.version",
                [SCHEMA_VERSION],
            )
            .await
            .map_err(database_error)?;

        Ok(connection)
    }
}

fn read_memory(row: &Row) -> Result<AgentMemory, StorageError> {
    let source = row
        .get::<Option<i64>>(4)
        .map_err(database_error)?
        .map(|id| u64::try_from(id).map(ConversationId))
        .transpose()
        .map_err(super::invalid)?;
    Ok(AgentMemory {
        id: row.get(0).map_err(database_error)?,
        kind: parse_memory_kind(&row.get::<String>(1).map_err(database_error)?)?,
        state: parse_memory_state(&row.get::<String>(2).map_err(database_error)?)?,
        content: row.get(3).map_err(database_error)?,
        source_conversation_id: source,
        confidence: row.get(5).map_err(database_error)?,
        created_at: Timestamp(row.get(6).map_err(database_error)?),
        updated_at: Timestamp(row.get(7).map_err(database_error)?),
    })
}

fn database_error(error: turso::Error) -> StorageError {
    StorageError::new(StorageErrorKind::Unavailable, error)
}
const fn memory_kind(value: MemoryKind) -> &'static str {
    match value {
        MemoryKind::Fact => "fact",
        MemoryKind::Preference => "preference",
        MemoryKind::Decision => "decision",
        MemoryKind::Procedure => "procedure",
    }
}
fn parse_memory_kind(value: &str) -> Result<MemoryKind, StorageError> {
    match value {
        "fact" => Ok(MemoryKind::Fact),
        "preference" => Ok(MemoryKind::Preference),
        "decision" => Ok(MemoryKind::Decision),
        "procedure" => Ok(MemoryKind::Procedure),
        _ => Err(super::failure(
            StorageErrorKind::InvalidData,
            "unknown memory kind",
        )),
    }
}
const fn memory_state(value: MemoryState) -> &'static str {
    match value {
        MemoryState::Candidate => "candidate",
        MemoryState::Active => "active",
        MemoryState::Rejected => "rejected",
    }
}
fn parse_memory_state(value: &str) -> Result<MemoryState, StorageError> {
    match value {
        "candidate" => Ok(MemoryState::Candidate),
        "active" => Ok(MemoryState::Active),
        "rejected" => Ok(MemoryState::Rejected),
        _ => Err(super::failure(
            StorageErrorKind::InvalidData,
            "unknown memory state",
        )),
    }
}
const fn session_state(value: AgentSessionState) -> &'static str {
    match value {
        AgentSessionState::Running => "running",
        AgentSessionState::AwaitingReview => "awaiting-review",
        AgentSessionState::Applied => "applied",
        AgentSessionState::Discarded => "discarded",
        AgentSessionState::Failed => "failed",
    }
}
fn parse_session_state(value: &str) -> Result<AgentSessionState, StorageError> {
    match value {
        "running" => Ok(AgentSessionState::Running),
        "awaiting-review" => Ok(AgentSessionState::AwaitingReview),
        "applied" => Ok(AgentSessionState::Applied),
        "discarded" => Ok(AgentSessionState::Discarded),
        "failed" => Ok(AgentSessionState::Failed),
        _ => Err(super::failure(
            StorageErrorKind::InvalidData,
            "unknown agent session state",
        )),
    }
}
fn normalize(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}
fn terms(value: &str) -> Vec<String> {
    normalize(value)
        .split_whitespace()
        .filter(|term| term.len() > 1)
        .map(ToOwned::to_owned)
        .collect()
}
fn lexical_score(terms: &[String], content: &str) -> f64 {
    if terms.is_empty() {
        return 0.0;
    }
    let hits = terms
        .iter()
        .filter(|term| content.contains(term.as_str()))
        .count();
    let hits = f64::from(u32::try_from(hits).unwrap_or(u32::MAX));
    let term_count = f64::from(u32::try_from(terms.len()).unwrap_or(u32::MAX));

    hits / term_count
}
fn encode_embedding(values: &[f32]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}
fn decode_embedding(bytes: &[u8]) -> Option<Vec<f32>> {
    if !bytes.len().is_multiple_of(4) {
        return None;
    }
    Some(
        bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|part| f32::from_le_bytes(*part))
            .collect(),
    )
}
fn cosine(left: &[f32], right: &[f32]) -> f64 {
    if left.len() != right.len() || left.is_empty() {
        return 0.0;
    }
    let (mut dot, mut a, mut b) = (0.0_f64, 0.0_f64, 0.0_f64);
    for (left, right) in left.iter().zip(right) {
        let left = f64::from(*left);
        let right = f64::from(*right);
        dot = left.mul_add(right, dot);
        a = left.mul_add(left, a);
        b = right.mul_add(right, b);
    }
    if a == 0.0 || b == 0.0 {
        0.0
    } else {
        f64::midpoint(dot / (a.sqrt() * b.sqrt()), 1.0)
    }
}
