//! Project-scoped durable data used to augment agent runs.
//!
//! These contracts deliberately expose domain values rather than Turso rows,
//! vector encodings, or filesystem implementation details.

use std::{future::Future, path::PathBuf, pin::Pin};

use crate::{ConversationId, StorageError, Timestamp};

pub type AgentDataFuture<T> =
    Pin<Box<dyn Future<Output = Result<T, StorageError>> + Send + 'static>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemoryKind {
    Fact,
    Preference,
    Decision,
    Procedure,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemoryState {
    Candidate,
    Active,
    Rejected,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AgentMemory {
    pub id: i64,
    pub kind: MemoryKind,
    pub state: MemoryState,
    pub content: String,
    pub source_conversation_id: Option<ConversationId>,
    pub confidence: f64,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NewAgentMemory {
    pub kind: MemoryKind,
    pub state: MemoryState,
    pub content: String,
    pub source_conversation_id: Option<ConversationId>,
    pub confidence: f64,
    pub embedding: Option<Vec<f32>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MemoryMatch {
    pub memory: AgentMemory,
    pub score: f64,
}

pub trait AgentMemoryStore: Send + Sync {
    fn remember(
        &self,
        project_root: PathBuf,
        memory: NewAgentMemory,
    ) -> AgentDataFuture<AgentMemory>;
    fn review_memory(
        &self,
        project_root: PathBuf,
        id: i64,
        state: MemoryState,
    ) -> AgentDataFuture<()>;
    fn memory_candidates(&self, project_root: PathBuf) -> AgentDataFuture<Vec<AgentMemory>>;
    fn recall(
        &self,
        project_root: PathBuf,
        query: String,
        embedding: Option<Vec<f32>>,
        limit: usize,
    ) -> AgentDataFuture<Vec<MemoryMatch>>;
}

#[derive(Clone, Debug, PartialEq)]
pub struct CodeChunk {
    pub path: String,
    pub language: String,
    pub symbol: Option<String>,
    pub start_line: u32,
    pub end_line: u32,
    pub content: String,
    pub content_hash: String,
    pub embedding: Option<Vec<f32>>,
    pub session_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CodeMatch {
    pub chunk: CodeChunk,
    pub score: f64,
}

pub trait CodeIndex: Send + Sync {
    fn file_hash(&self, project_root: PathBuf, path: String) -> AgentDataFuture<Option<String>>;
    fn replace_file(
        &self,
        project_root: PathBuf,
        path: String,
        content_hash: String,
        chunks: Vec<CodeChunk>,
    ) -> AgentDataFuture<()>;
    fn remove_file(&self, project_root: PathBuf, path: String) -> AgentDataFuture<()>;
    fn search_code(
        &self,
        project_root: PathBuf,
        query: String,
        embedding: Option<Vec<f32>>,
        session_id: Option<String>,
        limit: usize,
    ) -> AgentDataFuture<Vec<CodeMatch>>;
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CodeIndexReport {
    pub indexed_files: usize,
    pub unchanged_files: usize,
    pub removed_files: usize,
}

pub trait CodeIndexMaintainer: Send + Sync {
    fn refresh(&self, project_root: PathBuf) -> AgentDataFuture<CodeIndexReport>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CachedContent {
    pub path: String,
    pub content_hash: String,
    pub version: i64,
    pub content: String,
    pub compact_context: String,
    pub byte_size: u64,
    pub accessed_at: Timestamp,
}

pub trait AgentContentCache: Send + Sync {
    fn cached_content(
        &self,
        project_root: PathBuf,
        path: String,
        content_hash: String,
    ) -> AgentDataFuture<Option<CachedContent>>;
    fn cache_content(&self, project_root: PathBuf, content: CachedContent) -> AgentDataFuture<()>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentSessionState {
    Running,
    AwaitingReview,
    Applied,
    Discarded,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentSession {
    pub id: String,
    pub conversation_id: ConversationId,
    pub agentfs_path: PathBuf,
    pub state: AgentSessionState,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

pub trait AgentSessionStore: Send + Sync {
    fn create_session(&self, project_root: PathBuf, session: AgentSession) -> AgentDataFuture<()>;
    fn set_session_state(
        &self,
        project_root: PathBuf,
        id: String,
        state: AgentSessionState,
    ) -> AgentDataFuture<()>;
    fn session(&self, project_root: PathBuf, id: String) -> AgentDataFuture<Option<AgentSession>>;
}

pub trait EmbeddingProvider: Send + Sync {
    fn dimensions(&self) -> usize;
    fn embed(&self, texts: Vec<String>) -> AgentDataFuture<Vec<Vec<f32>>>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RetrievedContextKind {
    Memory,
    Code,
    CachedContent,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RetrievedContextBlock {
    pub kind: RetrievedContextKind,
    pub source: String,
    pub content: String,
    pub score: f64,
}
