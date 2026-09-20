use super::{
    AssistantTrace, AssistantTraceEntry, AssistantTraceKind, AssistantTraceStatus, Connection,
    LegacyTrace, MessageId, Result, Timestamp, TransactionBehavior, as_u64, db, scalar_i64, trace,
};

pub(super) async fn migrate_v1_to_v2(connection: &mut Connection) -> Result<()> {
    let legacy_exists = scalar_i64(
        connection,
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='agent_activities'",
        (),
    )
    .await?
        != 0;
    let traces = if legacy_exists {
        Some(load_legacy_traces(connection).await?)
    } else {
        None
    };

    rebuild_v1_message_graph(connection).await?;

    if let Some(traces) = traces {
        persist_legacy_traces(connection, traces).await?;
    }

    Ok(())
}

pub(super) async fn migrate_v2_to_v3(connection: &Connection) -> Result<()> {
    connection
        .execute("ALTER TABLE messages ADD COLUMN command_id TEXT", ())
        .await
        .map_err(db)?;
    Ok(())
}

async fn load_legacy_traces(
    connection: &Connection,
) -> Result<std::collections::BTreeMap<i64, Vec<LegacyTrace>>> {
    let mut statement = connection
        .prepare(
            "SELECT r.assistant_message_id,a.call_id,a.tool_name,a.kind,a.status, \
             a.summary,a.detail,a.created_at,a.sequence \
             FROM agent_activities a \
             JOIN agent_runs r ON r.id=a.run_id \
             ORDER BY r.assistant_message_id,a.sequence",
        )
        .await
        .map_err(db)?;
    let mut rows = statement.query(()).await.map_err(db)?;
    let mut traces: std::collections::BTreeMap<i64, Vec<LegacyTrace>> =
        std::collections::BTreeMap::new();

    while let Some(row) = rows.next().await.map_err(db)? {
        let message_id: i64 = row.get(0).map_err(db)?;
        let call_id: String = row.get(1).map_err(db)?;
        let entries = traces.entry(message_id).or_default();
        let index = entries.iter().position(|entry| entry.call_id == call_id);
        let kind: String = row.get(3).map_err(db)?;
        let status: String = row.get(4).map_err(db)?;
        let summary: String = row.get(5).map_err(db)?;
        let detail: String = row.get(6).map_err(db)?;
        let created_at: i64 = row.get(7).map_err(db)?;
        let entry = index.map_or_else(
            || {
                entries.push(LegacyTrace {
                    call_id: call_id.clone(),
                    tool_name: row.get(2).map_err(db)?,
                    title: summary.clone(),
                    input: String::new(),
                    output: String::new(),
                    status: AssistantTraceStatus::Requested,
                    started_at: created_at,
                    finished_at: None,
                });
                Ok(entries.len() - 1)
            },
            Ok,
        )?;
        let entry = &mut entries[entry];
        match kind.as_str() {
            "tool-call" => {
                entry.input = detail;
                entry.title = summary;
            }
            "approval-requested" => {
                entry.status = AssistantTraceStatus::AwaitingApproval;
                if entry.input.is_empty() {
                    entry.input = detail;
                }
                entry.title = summary;
            }
            "tool-result" => {
                entry.output = detail;
                entry.status =
                    if status == "failed" && entry.output.to_ascii_lowercase().contains("reject") {
                        AssistantTraceStatus::Rejected
                    } else if status == "failed" {
                        AssistantTraceStatus::Failed
                    } else {
                        AssistantTraceStatus::Completed
                    };
                entry.finished_at = Some(created_at);
            }
            _ => {}
        }
    }

    Ok(traces)
}

async fn persist_legacy_traces(
    connection: &mut Connection,
    traces: std::collections::BTreeMap<i64, Vec<LegacyTrace>>,
) -> Result<()> {
    for (message_id, entries) in traces {
        let trace = AssistantTrace {
            entries: entries
                .into_iter()
                .enumerate()
                .map(|(sequence, entry)| AssistantTraceEntry {
                    key: format!("tool:{}", entry.call_id),
                    sequence: sequence as u64,
                    kind: AssistantTraceKind::Tool,
                    status: entry.status,
                    title: entry.title,
                    tool_name: Some(entry.tool_name),
                    input: entry.input,
                    output: entry.output,
                    started_at: Some(Timestamp(entry.started_at)),
                    finished_at: entry.finished_at.map(Timestamp),
                })
                .collect(),
            thinking_duration_ms: None,
        };
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .await
            .map_err(db)?;
        trace::replace_trace(&transaction, MessageId(as_u64(message_id)?), &trace).await?;
        transaction.commit().await.map_err(db)?;
    }
    Ok(())
}

async fn rebuild_v1_message_graph(connection: &Connection) -> Result<()> {
    // Turso 0.4.4 drops table-level UNIQUE constraints when ADD COLUMN rewrites
    // a table's CREATE statement. Rebuild the small message graph instead so
    // the persisted schema retains its uniqueness and remains reopenable.
    connection
        .execute_batch(
            r"
            ALTER TABLE messages RENAME TO messages_v1;
            DROP TABLE IF EXISTS assistant_traces;

            DROP TABLE IF EXISTS agent_activities;

            ALTER TABLE agent_runs RENAME TO agent_runs_v1;
            ALTER TABLE attachments RENAME TO attachments_v1;

            CREATE TABLE messages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                conversation_id INTEGER NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
                sequence INTEGER NOT NULL,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                status TEXT NOT NULL,
                generation TEXT NOT NULL,
                outcome TEXT,
                failure TEXT,
                omitted_context_messages INTEGER NOT NULL DEFAULT 0,
                thinking_duration_ms INTEGER,
                created_at INTEGER NOT NULL
            );

            INSERT INTO messages(
                id, conversation_id, sequence, role, content, status, generation,
                outcome, failure, omitted_context_messages, thinking_duration_ms, created_at
            )
            SELECT
                id, conversation_id, sequence, role, content, status, generation,
                outcome, failure, omitted_context_messages, NULL, created_at
            FROM messages_v1;

            CREATE TABLE agent_runs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                conversation_id INTEGER NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
                assistant_message_id INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
                status TEXT NOT NULL,
                started_at INTEGER NOT NULL,
                finished_at INTEGER
            );

            INSERT INTO agent_runs(
                id, conversation_id, assistant_message_id, status, started_at, finished_at
            )
            SELECT id, conversation_id, assistant_message_id, status, started_at, finished_at
            FROM agent_runs_v1;

            CREATE TABLE assistant_traces (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                assistant_message_id INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
                trace_key TEXT NOT NULL,
                sequence INTEGER NOT NULL,
                kind TEXT NOT NULL,
                status TEXT NOT NULL,
                title TEXT NOT NULL,
                tool_name TEXT,
                input TEXT NOT NULL,
                output TEXT NOT NULL,
                started_at INTEGER,
                finished_at INTEGER
            );
            CREATE TABLE attachments (
                message_id INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
                position INTEGER NOT NULL,
                name TEXT NOT NULL,
                source_path BLOB NOT NULL,
                mime_type TEXT NOT NULL,
                byte_size INTEGER NOT NULL,
                managed INTEGER NOT NULL
            );

            INSERT INTO attachments(
                message_id, position, name, source_path, mime_type, byte_size, managed
            )
            SELECT message_id, position, name, source_path, mime_type, byte_size, managed
            FROM attachments_v1;

            DROP TABLE agent_runs_v1;
            DROP TABLE attachments_v1;
            DROP TABLE messages_v1;

            CREATE UNIQUE INDEX message_conversation_sequence
                ON messages(conversation_id, sequence);
            CREATE UNIQUE INDEX one_stream_per_conversation
                ON messages(conversation_id) WHERE status = 'streaming';
            CREATE INDEX message_conversation_order
                ON messages(conversation_id, sequence);
            CREATE UNIQUE INDEX agent_run_assistant_message
                ON agent_runs(assistant_message_id);
            CREATE UNIQUE INDEX assistant_trace_key
                ON assistant_traces(assistant_message_id, trace_key);
            CREATE UNIQUE INDEX assistant_trace_sequence
                ON assistant_traces(assistant_message_id, sequence);
            CREATE INDEX assistant_trace_order
                ON assistant_traces(assistant_message_id, sequence);
            CREATE UNIQUE INDEX attachment_position
                ON attachments(message_id, position);
            ",
        )
        .await
        .map_err(db)
}
