use super::{
    AgentContentCache, AgentDataFuture, CachedContent, MAX_CACHE_BYTES,
    MAX_CACHE_VERSIONS_PER_PATH, PathBuf, Timestamp, TursoAgentDatabase, database_error, params,
};

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

            let accessed_at = crate::now()?;

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
                    .map_err(crate::invalid)?,
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
                        i64::try_from(content.byte_size).map_err(crate::invalid)?,
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
