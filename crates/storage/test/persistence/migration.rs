use super::*;

#[test]
fn version_four_migration_backfills_full_text_indexes() {
    smol::block_on(async {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("history.sqlite3");
        let store = SqliteConversationStore::new(path.clone());
        store.initialize().await.unwrap();
        let pending = store.begin_turn(input(None)).await.unwrap();
        drop(store);

        let connection = rusqlite::Connection::open(&path).unwrap();
        connection
            .execute_batch(
                r"
                    DROP TRIGGER conversation_fts_insert;
                    DROP TRIGGER conversation_fts_delete;
                    DROP TRIGGER conversation_fts_update;
                    DROP TRIGGER message_fts_insert;
                    DROP TRIGGER message_fts_delete;
                    DROP TRIGGER message_fts_update;
                    DROP TABLE conversation_fts;
                    DROP TABLE message_fts;
                    DROP INDEX assistant_trace_order;
                    DROP TABLE assistant_traces;
                    CREATE TABLE agent_activities (
                        id INTEGER PRIMARY KEY AUTOINCREMENT,
                        run_id INTEGER NOT NULL REFERENCES agent_runs(id) ON DELETE CASCADE,
                        sequence INTEGER NOT NULL,
                        kind TEXT NOT NULL,
                        call_id TEXT NOT NULL,
                        tool_name TEXT NOT NULL,
                        status TEXT NOT NULL,
                        summary TEXT NOT NULL,
                        detail TEXT NOT NULL,
                        created_at INTEGER NOT NULL,
                        UNIQUE (run_id, sequence)
                    );
                    CREATE INDEX agent_activity_order ON agent_activities(run_id, sequence);
                    ALTER TABLE messages DROP COLUMN omitted_context_messages;
                    ALTER TABLE messages DROP COLUMN failure;
                    ALTER TABLE messages DROP COLUMN thinking_duration_ms;
                    PRAGMA user_version = 4;
                ",
            )
            .unwrap();
        drop(connection);

        let migrated = SqliteConversationStore::new(path);
        migrated.initialize().await.unwrap();
        let matches = migrated.search("main".into(), 10).await.unwrap();
        assert_eq!(matches[0].conversation_id, pending.conversation.id);
        assert_eq!(matches[0].message_id, Some(pending.user_message.id));
    });
}
