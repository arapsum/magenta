use super::{PathBuf, Project, ProjectStore, StorageFuture, Timestamp, TursoAppStore, db, params};

impl ProjectStore for TursoAppStore {
    fn projects(&self) -> StorageFuture<Vec<Project>> {
        self.run(async |connection| {
            let mut statement = connection
                .prepare(
                    "SELECT name,root,added_at,last_opened_at \
                     FROM projects \
                     ORDER BY last_opened_at DESC,name COLLATE NOCASE",
                )
                .await
                .map_err(db)?;
            let mut rows = statement.query(()).await.map_err(db)?;
            let mut result = Vec::new();

            while let Some(row) = rows.next().await.map_err(db)? {
                result.push(Project {
                    name: row.get(0).map_err(db)?,
                    root: super::records::decode_path(row.get(1).map_err(db)?)?,
                    added_at: Timestamp(row.get(2).map_err(db)?),
                    last_opened_at: Timestamp(row.get(3).map_err(db)?),
                });
            }

            Ok(result)
        })
    }

    fn upsert_project(&self, project: Project) -> StorageFuture<()> {
        self.run(async move |connection| {
            connection
                .execute(
                    "INSERT INTO projects(root,name,added_at,last_opened_at) \
                     VALUES (?1,?2,?3,?4) \
                     ON CONFLICT(root) DO UPDATE SET \
                     name=excluded.name,last_opened_at=excluded.last_opened_at",
                    params![
                        super::records::encode_path(&project.root),
                        project.name,
                        project.added_at.0,
                        project.last_opened_at.0,
                    ],
                )
                .await
                .map_err(db)?;

            Ok(())
        })
    }

    fn remove_project(&self, root: PathBuf) -> StorageFuture<()> {
        self.run(async move |connection| {
            connection
                .execute(
                    "DELETE FROM projects WHERE root=?1",
                    [super::records::encode_path(&root)],
                )
                .await
                .map_err(db)?;

            Ok(())
        })
    }
}
