use magenta_core::{
    ConversationId, ConversationSearchResult, MessageId, MessageSequence, Timestamp,
};
use rusqlite::{Connection, params};

use super::{Result, database_error, invalid};

pub fn search(
    connection: &Connection,
    query: &str,
    limit: usize,
) -> Result<Vec<ConversationSearchResult>> {
    let Some(query) = fts_query(query) else {
        return Ok(Vec::new());
    };
    let limit = i64::try_from(limit.min(100)).map_err(invalid)?;
    let result_limit = usize::try_from(limit).map_err(invalid)?;
    let mut candidates = title_search_results(connection, &query, limit)?;
    candidates.extend(message_search_results(connection, &query, limit)?);
    candidates.sort_by(|left, right| {
        left.1
            .cmp(&right.1)
            .then_with(|| left.2.total_cmp(&right.2))
            .then_with(|| right.0.updated_at.cmp(&left.0.updated_at))
    });
    let mut seen = std::collections::HashSet::new();
    Ok(candidates
        .into_iter()
        .filter_map(|(result, _, _)| seen.insert(result.conversation_id).then_some(result))
        .take(result_limit)
        .collect())
}

type RankedSearchResult = (ConversationSearchResult, u8, f64);

fn title_search_results(
    connection: &Connection,
    query: &str,
    limit: i64,
) -> Result<Vec<RankedSearchResult>> {
    let mut title_statement = connection
        .prepare(
            r"
                SELECT conversation.id,
                       highlight(conversation_fts, 0, char(1), char(2)),
                       COALESCE(
                           (
                               SELECT substr(trim(message.content), 1, 240)
                               FROM messages AS message
                               WHERE message.conversation_id = conversation.id
                                 AND message.role = 'user'
                                 AND trim(message.content) <> ''
                               ORDER BY message.sequence DESC
                               LIMIT 1
                           ),
                           ''
                       ),
                       conversation.updated_at,
                       bm25(conversation_fts)
                FROM conversation_fts
                INNER JOIN conversations AS conversation
                    ON conversation.id = conversation_fts.rowid
                WHERE conversation_fts MATCH ?1
                ORDER BY bm25(conversation_fts), conversation.updated_at DESC
                LIMIT ?2
            ",
        )
        .map_err(database_error)?;
    let title_rows = title_statement
        .query_map(params![query, limit], |row| {
            let marked_title = row.get::<_, String>(1)?;
            let (title, title_highlights) = marked_text(&marked_title);
            Ok((
                ConversationSearchResult {
                    conversation_id: ConversationId(row.get(0)?),
                    message_id: None,
                    message_sequence: None,
                    title,
                    title_highlights,
                    snippet: row.get(2)?,
                    snippet_highlights: Vec::new(),
                    updated_at: Timestamp(row.get(3)?),
                },
                0_u8,
                row.get::<_, f64>(4)?,
            ))
        })
        .map_err(database_error)?;
    title_rows
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(database_error)
}

fn message_search_results(
    connection: &Connection,
    query: &str,
    limit: i64,
) -> Result<Vec<RankedSearchResult>> {
    let mut message_statement = connection
        .prepare(
            r"
                SELECT conversation.id, message.id, message.sequence, conversation.title,
                       snippet(message_fts, 0, char(1), char(2), ' … ', 24),
                       conversation.updated_at, bm25(message_fts)
                FROM message_fts
                INNER JOIN messages AS message ON message.id = message_fts.rowid
                INNER JOIN conversations AS conversation
                    ON conversation.id = message.conversation_id
                WHERE message_fts MATCH ?1
                ORDER BY bm25(message_fts), conversation.updated_at DESC
                LIMIT ?2
            ",
        )
        .map_err(database_error)?;
    let message_rows = message_statement
        .query_map(params![query, limit], |row| {
            let marked_snippet = row.get::<_, String>(4)?;
            let (snippet, snippet_highlights) = marked_text(&marked_snippet);
            Ok((
                ConversationSearchResult {
                    conversation_id: ConversationId(row.get(0)?),
                    message_id: Some(MessageId(row.get(1)?)),
                    message_sequence: Some(MessageSequence(row.get(2)?)),
                    title: row.get(3)?,
                    title_highlights: Vec::new(),
                    snippet,
                    snippet_highlights,
                    updated_at: Timestamp(row.get(5)?),
                },
                1_u8,
                row.get::<_, f64>(6)?,
            ))
        })
        .map_err(database_error)?;
    message_rows
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(database_error)
}

fn fts_query(query: &str) -> Option<String> {
    let terms = query
        .split_whitespace()
        .filter(|term| !term.is_empty())
        .map(|term| format!("\"{}\"*", term.replace('"', "\"\"")))
        .collect::<Vec<_>>();
    (!terms.is_empty()).then(|| terms.join(" AND "))
}

fn marked_text(marked: &str) -> (String, Vec<std::ops::Range<usize>>) {
    let mut plain = String::with_capacity(marked.len());
    let mut highlights = Vec::new();
    let mut start = None;
    for character in marked.chars() {
        match character {
            '\u{1}' => start = Some(plain.len()),
            '\u{2}' => {
                if let Some(start) = start.take()
                    && start < plain.len()
                {
                    highlights.push(start..plain.len());
                }
            }
            _ => plain.push(character),
        }
    }
    if let Some(start) = start
        && start < plain.len()
    {
        highlights.push(start..plain.len());
    }
    (plain, highlights)
}
