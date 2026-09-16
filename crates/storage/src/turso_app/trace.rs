use super::{
    AssistantTrace, AssistantTraceEntry, Connection, MessageId, Result, Timestamp, Transaction,
    as_i64, db, params, parse_trace_kind, parse_trace_status, trace_kind, trace_status,
};

pub(super) async fn read_trace(
    connection: &Connection,
    id: MessageId,
    thinking_duration_ms: Option<i64>,
) -> Result<AssistantTrace> {
    let mut statement = connection
        .prepare(
            "SELECT trace_key,sequence,kind,status,title,tool_name,input,output,started_at,finished_at \
             FROM assistant_traces \
             WHERE assistant_message_id=?1 \
             ORDER BY sequence",
        )
        .await
        .map_err(db)?;
    let mut rows = statement.query([as_i64(id.0)?]).await.map_err(db)?;

    let mut entries = Vec::new();

    while let Some(row) = rows.next().await.map_err(db)? {
        entries.push(AssistantTraceEntry {
            key: row.get(0).map_err(db)?,
            sequence: u64::try_from(row.get::<i64>(1).map_err(db)?).map_err(super::invalid)?,
            kind: parse_trace_kind(&row.get::<String>(2).map_err(db)?)?,
            status: parse_trace_status(&row.get::<String>(3).map_err(db)?)?,
            title: row.get(4).map_err(db)?,
            tool_name: row.get(5).map_err(db)?,
            input: row.get(6).map_err(db)?,
            output: row.get(7).map_err(db)?,
            started_at: row.get::<Option<i64>>(8).map_err(db)?.map(Timestamp),
            finished_at: row.get::<Option<i64>>(9).map_err(db)?.map(Timestamp),
        });
    }

    Ok(AssistantTrace {
        entries,
        thinking_duration_ms: thinking_duration_ms
            .map(|value| u64::try_from(value).map_err(super::invalid))
            .transpose()?,
    })
}

pub(super) async fn replace_trace(
    transaction: &Transaction<'_>,
    message_id: MessageId,
    trace: &AssistantTrace,
) -> Result<()> {
    transaction
        .execute(
            "DELETE FROM assistant_traces WHERE assistant_message_id=?1",
            [as_i64(message_id.0)?],
        )
        .await
        .map_err(db)?;

    for entry in &trace.entries {
        transaction
            .execute(
                "INSERT INTO assistant_traces( \
                 assistant_message_id,trace_key,sequence,kind,status,title,tool_name,input,output, \
                 started_at,finished_at \
                 ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
                params![
                    as_i64(message_id.0)?,
                    entry.key.clone(),
                    as_i64(entry.sequence)?,
                    trace_kind(entry.kind),
                    trace_status(entry.status),
                    entry.title.clone(),
                    entry.tool_name.clone(),
                    entry.input.clone(),
                    entry.output.clone(),
                    entry.started_at.map(|value| value.0),
                    entry.finished_at.map(|value| value.0),
                ],
            )
            .await
            .map_err(db)?;
    }
    Ok(())
}
