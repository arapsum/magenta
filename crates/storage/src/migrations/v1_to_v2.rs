use rusqlite::Transaction;

use crate::{Result, database_error};

pub(super) fn apply(transaction: &Transaction<'_>) -> Result<()> {
    transaction
        .execute_batch(
            r"
                ALTER TABLE attachments
                    ADD COLUMN mime_type TEXT NOT NULL DEFAULT 'application/octet-stream';
                ALTER TABLE attachments
                    ADD COLUMN byte_size INTEGER NOT NULL DEFAULT 0 CHECK (byte_size >= 0);
                ALTER TABLE attachments
                    ADD COLUMN managed INTEGER NOT NULL DEFAULT 0 CHECK (managed IN (0, 1));
                PRAGMA user_version = 2;
            ",
        )
        .map_err(database_error)
}
