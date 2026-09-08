mod v1_to_v2;
mod v2_to_v3;
mod v3_to_v4;
mod v4_to_v5;
mod v5_to_v6;

use magenta_core::StorageErrorKind;
use rusqlite::Transaction;

use crate::{Result, failure};

pub fn apply(version: i64, transaction: &Transaction<'_>) -> Result<()> {
    match version {
        1 => {
            v1_to_v2::apply(transaction)?;
            v2_to_v3::apply(transaction)?;
            v3_to_v4::apply(transaction)?;
            v4_to_v5::apply(transaction)?;
            v5_to_v6::apply(transaction)
        }
        2 => {
            v2_to_v3::apply(transaction)?;
            v3_to_v4::apply(transaction)?;
            v4_to_v5::apply(transaction)?;
            v5_to_v6::apply(transaction)
        }
        3 => {
            v3_to_v4::apply(transaction)?;
            v4_to_v5::apply(transaction)?;
            v5_to_v6::apply(transaction)
        }
        4 => {
            v4_to_v5::apply(transaction)?;
            v5_to_v6::apply(transaction)
        }
        5 => v5_to_v6::apply(transaction),
        6 => Ok(()),
        _ => Err(failure(
            StorageErrorKind::UnsupportedVersion,
            "unsupported database schema version",
        )),
    }
}
