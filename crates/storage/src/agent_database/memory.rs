use super::*;

impl AgentMemoryStore for TursoAgentDatabase {
    fn remember(
        &self,
        project_root: PathBuf,
        memory: NewAgentMemory,
    ) -> AgentDataFuture<AgentMemory> {
        let this = self.clone();
        Box::pin(async move {
            if memory.content.trim().is_empty() {
                return Err(crate::failure(
                    StorageErrorKind::InvalidData,
                    "memory is empty",
                ));
            }

            let connection = this.connect(&project_root).await?;
            let timestamp = crate::now()?;

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
                            .map_err(crate::invalid)?,
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
                    params![memory_state(state), crate::now()?, id],
                )
                .await
                .map_err(database_error)?;

            if changed == 0 {
                return Err(crate::failure(
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
