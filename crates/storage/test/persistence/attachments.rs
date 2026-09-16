use super::*;

#[test]
fn delete_removes_conversation_messages_and_attachment_records() {
    smol::block_on(async {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("history.sqlite3");
        let source = write_png(directory.path(), "source.png");
        let store = SqliteConversationStore::new(path.clone());
        store.initialize().await.unwrap();
        let pending = store
            .begin_turn(input_with_attachments(
                None,
                vec![AttachmentDraft {
                    name: "source.png".into(),
                    source_path: source.clone(),
                }],
            ))
            .await
            .unwrap();
        let id = pending.conversation.id;
        let imported = pending.user_message.attachments[0].clone();
        assert!(imported.managed);
        assert!(imported.path.exists());
        assert!(source.exists());

        let connection = rusqlite::Connection::open(&path).unwrap();
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM messages", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            2
        );
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM attachments", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap(),
            1
        );

        store.delete(id).await.unwrap();
        assert!(!imported.path.exists());
        assert!(source.exists());
        assert!(store.summaries().await.unwrap().is_empty());
        assert_eq!(
            store.load(id).await.unwrap_err().kind,
            StorageErrorKind::NotFound
        );
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM messages", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM attachments", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap(),
            0
        );
        assert_eq!(
            store.delete(ConversationId(999)).await.unwrap_err().kind,
            StorageErrorKind::NotFound
        );
    });
}
