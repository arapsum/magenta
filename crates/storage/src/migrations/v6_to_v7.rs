use rusqlite::Transaction;

use crate::{Result, database_error};

pub fn apply(transaction: &Transaction<'_>) -> Result<()> {
    transaction
        .execute_batch(
            r"
                ALTER TABLE messages ADD COLUMN failure TEXT;
                PRAGMA user_version = 7;
            ",
        )
        .map_err(database_error)
}
