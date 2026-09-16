use super::{
    AgentDataFuture, AgentSession, AgentSessionState, AgentSessionStore, ConversationId, PathBuf,
    StorageErrorKind, Timestamp, TursoAgentDatabase, database_error, params, parse_session_state,
    session_state,
};

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
                        i64::try_from(session.conversation_id.0).map_err(crate::invalid)?,
                        crate::records::encode_path(&session.agentfs_path),
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
                    params![session_state(state), crate::now()?, id],
                )
                .await
                .map_err(database_error)?;

            if changed == 0 {
                return Err(crate::failure(
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
                        .map_err(crate::invalid)?,
                ),
                agentfs_path: crate::records::decode_path(row.get(1).map_err(database_error)?)?,
                state: parse_session_state(&row.get::<String>(2).map_err(database_error)?)?,
                created_at: Timestamp(row.get(3).map_err(database_error)?),
                updated_at: Timestamp(row.get(4).map_err(database_error)?),
            }))
        })
    }
}
