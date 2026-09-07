use rusqlite::Transaction;

use crate::{Result, database_error};

pub(super) fn apply(transaction: &Transaction<'_>) -> Result<()> {
    transaction
        .execute_batch(
            r"
                CREATE TABLE projects (
                    root BLOB PRIMARY KEY,
                    name TEXT NOT NULL,
                    added_at INTEGER NOT NULL,
                    last_opened_at INTEGER NOT NULL
                );
                CREATE INDEX project_recency ON projects(last_opened_at DESC, name);
                PRAGMA user_version = 4;
            ",
        )
        .map_err(database_error)
}
