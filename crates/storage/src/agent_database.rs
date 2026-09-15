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

impl AgentMemoryStore for TursoAgentDatabase {
    fn remember(
        &self,
        project_root: PathBuf,
        memory: NewAgentMemory,
    ) -> AgentDataFuture<AgentMemory> {
        let this = self.clone();
        Box::pin(async move {
            if memory.content.trim().is_empty() {
                return Err(super::failure(
                    StorageErrorKind::InvalidData,
                    "memory is empty",
                ));
            }

            let connection = this.connect(&project_root).await?;
            let timestamp = super::now()?;

            let embedding = memory.embedding.as_deref().map(encode_embedding);

            connection
                .execute(
                    "INSERT INTO memories(kind, state, content, normalized_content, \
                 source_conversation_id, confidence, embedding, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)",
                    params![
                        memory_kind(memory.kind),
                        memory_state(memory.state),
                        memory.content.as_str(),
                        normalize(&memory.content),
                        memory
                            .source_conversation_id
                            .map(|id| i64::try_from(id.0))
                            .transpose()
                            .map_err(super::invalid)?,
                        memory.confidence.clamp(0.0, 1.0),
                        embedding,
                        timestamp,
                    ],
                )
                .await
                .map_err(database_error)?;

            Ok(AgentMemory {
                id: connection.last_insert_rowid(),
                kind: memory.kind,
                state: memory.state,
                content: memory.content,
                source_conversation_id: memory.source_conversation_id,
                confidence: memory.confidence.clamp(0.0, 1.0),
                created_at: Timestamp(timestamp),
                updated_at: Timestamp(timestamp),
            })
        })
    }

    fn review_memory(
        &self,
        project_root: PathBuf,
        id: i64,
        state: MemoryState,
    ) -> AgentDataFuture<()> {
        let this = self.clone();
        Box::pin(async move {
            let connection = this.connect(&project_root).await?;

            let changed = connection
                .execute(
                    "UPDATE memories SET state = ?1, updated_at = ?2 WHERE id = ?3",
                    params![memory_state(state), super::now()?, id],
                )
                .await
                .map_err(database_error)?;

            if changed == 0 {
                return Err(super::failure(
                    StorageErrorKind::NotFound,
                    "memory does not exist",
                ));
            }
            Ok(())
        })
    }

    fn memory_candidates(&self, project_root: PathBuf) -> AgentDataFuture<Vec<AgentMemory>> {
        let this = self.clone();
        Box::pin(async move {
            let connection = this.connect(&project_root).await?;

            let mut statement = connection.prepare(
                "SELECT id, kind, state, content, source_conversation_id, confidence, \
                 created_at, updated_at FROM memories WHERE state = 'candidate' ORDER BY updated_at DESC"
            ).await.map_err(database_error)?;

            let mut rows = statement.query(()).await.map_err(database_error)?;

            let mut result = Vec::new();

            while let Some(row) = rows.next().await.map_err(database_error)? {
                result.push(read_memory(&row)?);
            }

            Ok(result)
        })
    }

    fn recall(
        &self,
        project_root: PathBuf,
        query: String,
        embedding: Option<Vec<f32>>,
        limit: usize,
    ) -> AgentDataFuture<Vec<MemoryMatch>> {
        let this = self.clone();
        Box::pin(async move {
            let connection = this.connect(&project_root).await?;

            let mut statement = connection
                .prepare(
                    "SELECT id, kind, state, content, source_conversation_id, confidence, \
                 created_at, updated_at, normalized_content, embedding \
                 FROM memories WHERE state = 'active'",
                )
                .await
                .map_err(database_error)?;
            let mut rows = statement.query(()).await.map_err(database_error)?;

            let terms = terms(&query);

            let mut matches = Vec::new();

            while let Some(row) = rows.next().await.map_err(database_error)? {
                let normalized: String = row.get(8).map_err(database_error)?;
                let lexical = lexical_score(&terms, &normalized);

                let stored: Option<Vec<u8>> = row.get(9).map_err(database_error)?;
                let semantic = embedding
                    .as_deref()
                    .zip(stored.as_deref().and_then(decode_embedding))
                    .map_or(0.0, |(left, right)| cosine(left, &right));

                let memory = read_memory(&row)?;

                let score = 0.10f64
                    .mul_add(memory.confidence, 0.35f64.mul_add(lexical, 0.55 * semantic))
                    .clamp(0.0, 1.0);

                if score > 0.0 {
                    matches.push(MemoryMatch { memory, score });
                }
            }

            matches.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(Ordering::Equal));
            matches.truncate(limit.min(8));
            Ok(matches)
        })
    }
}

impl CodeIndex for TursoAgentDatabase {
    fn file_hash(&self, project_root: PathBuf, path: String) -> AgentDataFuture<Option<String>> {
        let this = self.clone();
        Box::pin(async move {
            let connection = this.connect(&project_root).await?;

            let mut statement = connection
                .prepare("SELECT content_hash FROM indexed_files WHERE path = ?1")
                .await
                .map_err(database_error)?;

            let mut rows = statement.query([path]).await.map_err(database_error)?;

            rows.next()
                .await
                .map_err(database_error)?
                .map(|row| row.get::<String>(0))
                .transpose()
                .map_err(database_error)
        })
    }

    fn replace_file(
        &self,
        project_root: PathBuf,
        path: String,
        content_hash: String,
        chunks: Vec<CodeChunk>,
    ) -> AgentDataFuture<()> {
        let this = self.clone();
        Box::pin(async move {
            let mut connection = this.connect(&project_root).await?;

            let transaction = connection
                .transaction_with_behavior(turso::transaction::TransactionBehavior::Immediate)
                .await
                .map_err(database_error)?;

            transaction
                .execute(
                    "DELETE FROM code_chunks WHERE path = ?1 AND session_id IS NULL",
                    [path.as_str()],
                )
                .await
                .map_err(database_error)?;

            for chunk in chunks {
                let normalized_content = normalize(&chunk.content);

                transaction
                    .execute(
                        "INSERT INTO code_chunks( \
                         path, language, symbol, start_line, end_line, content, \
                         normalized_content, content_hash, embedding, session_id \
                         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                        params![
                            chunk.path,
                            chunk.language,
                            chunk.symbol,
                            i64::from(chunk.start_line),
                            i64::from(chunk.end_line),
                            chunk.content,
                            normalized_content,
                            chunk.content_hash,
                            chunk.embedding.as_deref().map(encode_embedding),
                            chunk.session_id,
                        ],
                    )
                    .await
                    .map_err(database_error)?;
            }

            transaction
                .execute(
                    "INSERT INTO indexed_files(path, content_hash, indexed_at) \
                     VALUES (?1, ?2, ?3) \
                     ON CONFLICT(path) DO UPDATE SET \
                     content_hash = excluded.content_hash, \
                     indexed_at = excluded.indexed_at",
                    params![path, content_hash, super::now()?],
                )
                .await
                .map_err(database_error)?;

            transaction.commit().await.map_err(database_error)
        })
    }

    fn remove_file(&self, project_root: PathBuf, path: String) -> AgentDataFuture<()> {
        let this = self.clone();
        Box::pin(async move {
            let mut connection = this.connect(&project_root).await?;

            let transaction = connection
                .transaction_with_behavior(turso::transaction::TransactionBehavior::Immediate)
                .await
                .map_err(database_error)?;

            transaction
                .execute("DELETE FROM code_chunks WHERE path = ?1", [path.as_str()])
                .await
                .map_err(database_error)?;

            transaction
                .execute("DELETE FROM indexed_files WHERE path = ?1", [path.as_str()])
                .await
                .map_err(database_error)?;

            transaction.commit().await.map_err(database_error)
        })
    }

    fn search_code(
        &self,
        project_root: PathBuf,
        query: String,
        embedding: Option<Vec<f32>>,
        session_id: Option<String>,
        limit: usize,
    ) -> AgentDataFuture<Vec<CodeMatch>> {
        let this = self.clone();
        Box::pin(async move {
            let connection = this.connect(&project_root).await?;

            let mut statement = connection
                .prepare(
                    "SELECT path, language, symbol, start_line, end_line, content, \
                     content_hash, session_id, normalized_content, embedding \
                     FROM code_chunks \
                     WHERE session_id IS NULL OR session_id = ?1",
                )
                .await
                .map_err(database_error)?;

            let mut rows = statement
                .query([session_id])
                .await
                .map_err(database_error)?;
            let terms = terms(&query);

            let mut result = Vec::new();

            while let Some(row) = rows.next().await.map_err(database_error)? {
                let normalized: String = row.get(8).map_err(database_error)?;
                let lexical = lexical_score(&terms, &normalized);

                let stored: Option<Vec<u8>> = row.get(9).map_err(database_error)?;
                let semantic = embedding
                    .as_deref()
                    .zip(stored.as_deref().and_then(decode_embedding))
                    .map_or(0.0, |(left, right)| cosine(left, &right));

                let mut score = 0.45f64.mul_add(lexical, 0.55 * semantic);
                let symbol: Option<String> = row.get(2).map_err(database_error)?;

                if symbol
                    .as_ref()
                    .is_some_and(|value| normalize(value).contains(&normalize(&query)))
                {
                    score = (score + 0.25).min(1.0);
                }

                if score > 0.0 {
                    result.push(CodeMatch {
                        chunk: CodeChunk {
                            path: row.get(0).map_err(database_error)?,
                            language: row.get(1).map_err(database_error)?,
                            symbol,
                            start_line: u32::try_from(row.get::<i64>(3).map_err(database_error)?)
                                .map_err(super::invalid)?,
                            end_line: u32::try_from(row.get::<i64>(4).map_err(database_error)?)
                                .map_err(super::invalid)?,
                            content: row.get(5).map_err(database_error)?,
                            content_hash: row.get(6).map_err(database_error)?,
                            embedding: stored.as_deref().and_then(decode_embedding),
                            session_id: row.get(7).map_err(database_error)?,
                        },
                        score,
                    });
                }
            }

            result.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(Ordering::Equal));
            result.truncate(limit.min(50));
            Ok(result)
        })
    }
}

impl AgentContentCache for TursoAgentDatabase {
    fn cached_content(
        &self,
        project_root: PathBuf,
        path: String,
        content_hash: String,
    ) -> AgentDataFuture<Option<CachedContent>> {
        let this = self.clone();
        Box::pin(async move {
            let connection = this.connect(&project_root).await?;

            let mut statement = connection
                .prepare(
                    "SELECT version, content, compact_context, byte_size, accessed_at \
                     FROM content_cache \
                     WHERE path = ?1 AND content_hash = ?2",
                )
                .await
                .map_err(database_error)?;
            let mut rows = statement
                .query(params![path.as_str(), content_hash.as_str()])
                .await
                .map_err(database_error)?;
            let Some(row) = rows.next().await.map_err(database_error)? else {
                return Ok(None);
            };

            let accessed_at = super::now()?;

            connection
                .execute(
                    "UPDATE content_cache SET accessed_at = ?1 \
                     WHERE path = ?2 AND content_hash = ?3",
                    params![accessed_at, path.as_str(), content_hash.as_str(),],
                )
                .await
                .map_err(database_error)?;

            Ok(Some(CachedContent {
                path,
                content_hash,
                version: row.get(0).map_err(database_error)?,
                content: row.get(1).map_err(database_error)?,
                compact_context: row.get(2).map_err(database_error)?,
                byte_size: u64::try_from(row.get::<i64>(3).map_err(database_error)?)
                    .map_err(super::invalid)?,
                accessed_at: Timestamp(accessed_at),
            }))
        })
    }

    fn cache_content(&self, project_root: PathBuf, content: CachedContent) -> AgentDataFuture<()> {
        let this = self.clone();
        Box::pin(async move {
            let mut connection = this.connect(&project_root).await?;

            let transaction = connection
                .transaction_with_behavior(turso::transaction::TransactionBehavior::Immediate)
                .await
                .map_err(database_error)?;

            transaction
                .execute(
                    "INSERT INTO content_cache( \
                     path, content_hash, version, content, compact_context, \
                     byte_size, accessed_at \
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) \
                     ON CONFLICT(path, content_hash) DO UPDATE SET \
                     compact_context = excluded.compact_context, \
                     accessed_at = excluded.accessed_at",
                    params![
                        content.path.as_str(),
                        content.content_hash.as_str(),
                        content.version,
                        content.content.as_str(),
                        content.compact_context.as_str(),
                        i64::try_from(content.byte_size).map_err(super::invalid)?,
                        content.accessed_at.0,
                    ],
                )
                .await
                .map_err(database_error)?;

            transaction
                .execute(
                    "DELETE FROM content_cache \
                     WHERE path = ?1 AND id NOT IN ( \
                     SELECT id FROM content_cache WHERE path = ?1 \
                     ORDER BY version DESC LIMIT ?2)",
                    params![content.path.as_str(), MAX_CACHE_VERSIONS_PER_PATH],
                )
                .await
                .map_err(database_error)?;

            let mut total_statement = transaction
                .prepare("SELECT COALESCE(SUM(byte_size), 0) FROM content_cache")
                .await
                .map_err(database_error)?;
            let mut total_rows = total_statement.query(()).await.map_err(database_error)?;

            let total = total_rows
                .next()
                .await
                .map_err(database_error)?
                .map(|row| row.get::<i64>(0))
                .transpose()
                .map_err(database_error)?
                .unwrap_or(0);

            drop(total_rows);
            drop(total_statement);

            if total > MAX_CACHE_BYTES {
                transaction
                    .execute(
                        "DELETE FROM content_cache WHERE id IN ( \
                         SELECT id FROM content_cache \
                         ORDER BY accessed_at ASC LIMIT 32)",
                        (),
                    )
                    .await
                    .map_err(database_error)?;
            }

            transaction.commit().await.map_err(database_error)
        })
    }
}

impl AgentSessionStore for TursoAgentDatabase {
    fn create_session(&self, project_root: PathBuf, session: AgentSession) -> AgentDataFuture<()> {
        let this = self.clone();
        Box::pin(async move {
            let connection = this.connect(&project_root).await?;

            connection
                .execute(
                    "INSERT INTO agent_sessions( \
                     id, conversation_id, agentfs_path, state, created_at, updated_at \
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![
                        session.id,
                        i64::try_from(session.conversation_id.0).map_err(super::invalid)?,
                        super::records::encode_path(&session.agentfs_path),
                        session_state(session.state),
                        session.created_at.0,
                        session.updated_at.0,
                    ],
                )
                .await
                .map_err(database_error)?;

            Ok(())
        })
    }

    fn set_session_state(
        &self,
        project_root: PathBuf,
        id: String,
        state: AgentSessionState,
    ) -> AgentDataFuture<()> {
        let this = self.clone();
        Box::pin(async move {
            let connection = this.connect(&project_root).await?;

            let changed = connection
                .execute(
                    "UPDATE agent_sessions SET state = ?1, updated_at = ?2 WHERE id = ?3",
                    params![session_state(state), super::now()?, id],
                )
                .await
                .map_err(database_error)?;

            if changed == 0 {
                return Err(super::failure(
                    StorageErrorKind::NotFound,
                    "agent session does not exist",
                ));
            }
            Ok(())
        })
    }

    fn session(&self, project_root: PathBuf, id: String) -> AgentDataFuture<Option<AgentSession>> {
        let this = self.clone();
        Box::pin(async move {
            let connection = this.connect(&project_root).await?;

            let mut statement = connection
                .prepare(
                    "SELECT conversation_id, agentfs_path, state, created_at, updated_at \
                     FROM agent_sessions WHERE id = ?1",
                )
                .await
                .map_err(database_error)?;
            let mut rows = statement
                .query([id.as_str()])
                .await
                .map_err(database_error)?;
            let Some(row) = rows.next().await.map_err(database_error)? else {
                return Ok(None);
            };

            Ok(Some(AgentSession {
                id,
                conversation_id: ConversationId(
                    u64::try_from(row.get::<i64>(0).map_err(database_error)?)
                        .map_err(super::invalid)?,
                ),
                agentfs_path: super::records::decode_path(row.get(1).map_err(database_error)?)?,
                state: parse_session_state(&row.get::<String>(2).map_err(database_error)?)?,
                created_at: Timestamp(row.get(3).map_err(database_error)?),
                updated_at: Timestamp(row.get(4).map_err(database_error)?),
            }))
        })
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
