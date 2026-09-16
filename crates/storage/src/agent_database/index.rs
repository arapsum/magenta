use super::*;

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
                    params![path, content_hash, crate::now()?],
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
                                .map_err(crate::invalid)?,
                            end_line: u32::try_from(row.get::<i64>(4).map_err(database_error)?)
                                .map_err(crate::invalid)?,
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
