mod v1_to_v2;
mod v2_to_v3;

use magenta_core::StorageErrorKind;
use rusqlite::Transaction;

use crate::{Result, failure};

pub fn apply(version: i64, transaction: &Transaction<'_>) -> Result<()> {
    match version {
        1 => {
            v1_to_v2::apply(transaction)?;
            v2_to_v3::apply(transaction)
        }
        2 => v2_to_v3::apply(transaction),
        3 => Ok(()),
        _ => Err(failure(
            StorageErrorKind::UnsupportedVersion,
            "unsupported database schema version",
        )),
    }
}
