use magenta_core::ConversationId;
use rusqlite::Connection;

use super::{Result, database_error, invalid, records};

pub fn for_conversation(
    connection: &Connection,
    id: ConversationId,
) -> Result<Vec<magenta_core::Attachment>> {
    let mut statement = connection
        .prepare(
            r"
                SELECT attachment.name, attachment.source_path, attachment.mime_type,
                       attachment.byte_size, attachment.managed
                FROM attachments AS attachment
                INNER JOIN messages AS message ON message.id = attachment.message_id
                WHERE message.conversation_id = ?1
                  AND attachment.managed = 1
                ORDER BY message.sequence, attachment.position
            ",
        )
        .map_err(database_error)?;
    let rows = statement
        .query_map([id.0], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Vec<u8>>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, bool>(4)?,
            ))
        })
        .map_err(database_error)?;
    rows.map(|row| {
        let (name, path, mime_type, byte_size, managed) = row.map_err(database_error)?;
        Ok(magenta_core::Attachment {
            name,
            path: records::decode_path(path)?,
            mime_type,
            byte_size: u64::try_from(byte_size).map_err(invalid)?,
            managed,
        })
    })
    .collect()
}
