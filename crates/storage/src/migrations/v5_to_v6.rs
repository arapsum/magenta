use rusqlite::Transaction;

use crate::{Result, database_error};

pub fn apply(transaction: &Transaction<'_>) -> Result<()> {
    transaction
        .execute_batch(
            r"
                ALTER TABLE messages ADD COLUMN omitted_context_messages INTEGER NOT NULL
                    DEFAULT 0 CHECK (omitted_context_messages >= 0);
                PRAGMA user_version = 6;
            ",
        )
        .map_err(database_error)
}
