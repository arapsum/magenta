use rusqlite::Transaction;

use crate::{Result, database_error};

pub(super) fn apply(transaction: &Transaction<'_>) -> Result<()> {
    transaction
        .execute_batch(include_str!("../search_schema.sql"))
        .map_err(database_error)?;
    transaction
        .execute_batch(
            r"
                INSERT INTO conversation_fts(conversation_fts) VALUES('rebuild');
                INSERT INTO message_fts(message_fts) VALUES('rebuild');
                PRAGMA user_version = 5;
            ",
        )
        .map_err(database_error)
}
