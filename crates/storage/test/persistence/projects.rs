use super::*;

#[test]
fn projects_persist_and_forgetting_one_does_not_delete_conversations() {
    smol::block_on(async {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("history.sqlite3");
        let store = SqliteConversationStore::new(path.clone());
        store.initialize().await.unwrap();
        let pending = store.begin_turn(input(None)).await.unwrap();
        let project_root = directory.path().join("project");
        fs::create_dir(&project_root).unwrap();

        store
            .upsert_project(Project {
                name: "project".into(),
                root: project_root.clone(),
                added_at: Timestamp(1),
                last_opened_at: Timestamp(2),
            })
            .await
            .unwrap();
        assert_eq!(store.projects().await.unwrap()[0].root, project_root);

        store.remove_project(project_root).await.unwrap();
        assert!(store.projects().await.unwrap().is_empty());
        assert_eq!(store.summaries().await.unwrap().len(), 1);
        assert_eq!(
            store
                .load(pending.conversation.id)
                .await
                .unwrap()
                .conversation
                .id,
            pending.conversation.id
        );
    });
}
