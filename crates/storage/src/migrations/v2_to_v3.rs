use rusqlite::Transaction;

use crate::{Result, database_error};

pub(super) fn apply(transaction: &Transaction<'_>) -> Result<()> {
    transaction
        .execute_batch(
            r"
                ALTER TABLE conversations ADD COLUMN mode TEXT NOT NULL DEFAULT 'chat'
                    CHECK (mode IN ('chat', 'agent'));
                ALTER TABLE conversations ADD COLUMN workspace_root BLOB;
                CREATE TABLE agent_runs (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    conversation_id INTEGER NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
                    assistant_message_id INTEGER NOT NULL UNIQUE REFERENCES messages(id) ON DELETE CASCADE,
                    status TEXT NOT NULL CHECK (status IN ('running', 'completed', 'stopped', 'failed')),
                    started_at INTEGER NOT NULL,
                    finished_at INTEGER
                );
                CREATE TABLE agent_activities (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    run_id INTEGER NOT NULL REFERENCES agent_runs(id) ON DELETE CASCADE,
                    sequence INTEGER NOT NULL CHECK (sequence >= 0),
                    kind TEXT NOT NULL CHECK (kind IN ('tool-call', 'approval-requested', 'tool-result')),
                    call_id TEXT NOT NULL,
                    tool_name TEXT NOT NULL,
                    status TEXT NOT NULL,
                    summary TEXT NOT NULL,
                    detail TEXT NOT NULL,
                    created_at INTEGER NOT NULL,
                    UNIQUE (run_id, sequence)
                );
                CREATE INDEX agent_activity_order ON agent_activities(run_id, sequence);
                PRAGMA user_version = 3;
            ",
        )
        .map_err(database_error)
}
