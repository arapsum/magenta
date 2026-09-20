use rusqlite::Transaction;

use crate::{Result, database_error};

pub fn apply(transaction: &Transaction<'_>) -> Result<()> {
    transaction
        .execute("ALTER TABLE messages ADD COLUMN command_id TEXT", [])
        .map_err(database_error)?;
    transaction
        .execute("PRAGMA user_version = 9", [])
        .map_err(database_error)?;
    Ok(())
}
