use rusqlite::Transaction;

use crate::{Result, database_error};

pub fn apply(transaction: &Transaction<'_>) -> Result<()> {
    transaction
        .execute_batch(
            r"
                ALTER TABLE messages ADD COLUMN thinking_duration_ms INTEGER
                    CHECK (thinking_duration_ms IS NULL OR thinking_duration_ms >= 0);

                CREATE TABLE assistant_traces (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    assistant_message_id INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
                    trace_key TEXT NOT NULL,
                    sequence INTEGER NOT NULL CHECK (sequence >= 0),
                    kind TEXT NOT NULL CHECK (kind IN ('reasoning_summary', 'tool')),
                    status TEXT NOT NULL CHECK (status IN ('streaming', 'requested', 'running', 'awaiting_approval', 'completed', 'rejected', 'failed', 'stopped')),
                    title TEXT NOT NULL,
                    tool_name TEXT,
                    input TEXT NOT NULL,
                    output TEXT NOT NULL,
                    started_at INTEGER,
                    finished_at INTEGER,
                    UNIQUE (assistant_message_id, trace_key),
                    UNIQUE (assistant_message_id, sequence)
                );

                CREATE INDEX assistant_trace_order
                    ON assistant_traces(assistant_message_id, sequence);

                WITH grouped AS (
                    SELECT
                        run.assistant_message_id,
                        activity.call_id,
                        MIN(activity.sequence) AS first_sequence,
                        COALESCE(
                            MAX(CASE WHEN activity.kind = 'tool-call' THEN activity.tool_name END),
                            MAX(activity.tool_name)
                        ) AS tool_name,
                        COALESCE(
                            MAX(CASE WHEN activity.kind = 'tool-call' THEN activity.summary END),
                            'Tool'
                        ) AS title,
                        COALESCE(
                            MAX(CASE WHEN activity.kind = 'tool-call' THEN activity.detail END),
                            ''
                        ) AS input,
                        COALESCE(
                            MAX(CASE WHEN activity.kind = 'tool-result' THEN activity.detail END),
                            ''
                        ) AS output,
                        MIN(activity.created_at) AS started_at,
                        MAX(CASE WHEN activity.kind = 'tool-result' THEN activity.created_at END) AS finished_at,
                        CASE
                            WHEN MAX(CASE WHEN activity.kind = 'tool-result'
                                          AND activity.status = 'failed'
                                          AND lower(activity.detail) LIKE '%reject%'
                                          THEN 1 ELSE 0 END) = 1 THEN 'rejected'
                            WHEN MAX(CASE WHEN activity.kind = 'tool-result'
                                          AND activity.status = 'failed'
                                          THEN 1 ELSE 0 END) = 1 THEN 'failed'
                            WHEN MAX(CASE WHEN activity.kind = 'tool-result' THEN 1 ELSE 0 END) = 1 THEN 'completed'
                            WHEN MAX(CASE WHEN activity.kind = 'approval-requested' THEN 1 ELSE 0 END) = 1 THEN 'awaiting_approval'
                            ELSE 'requested'
                        END AS status
                    FROM agent_activities AS activity
                    INNER JOIN agent_runs AS run ON run.id = activity.run_id
                    GROUP BY run.assistant_message_id, activity.call_id
                ), ordered AS (
                    SELECT *, ROW_NUMBER() OVER (
                        PARTITION BY assistant_message_id ORDER BY first_sequence
                    ) - 1 AS trace_sequence
                    FROM grouped
                )
                INSERT INTO assistant_traces(
                    assistant_message_id, trace_key, sequence, kind, status, title,
                    tool_name, input, output, started_at, finished_at
                )
                SELECT
                    assistant_message_id,
                    'tool:' || call_id,
                    trace_sequence,
                    'tool', status, title, tool_name, input, output, started_at, finished_at
                FROM ordered;

                DROP INDEX agent_activity_order;
                DROP TABLE agent_activities;
                PRAGMA user_version = 8;
            ",
        )
        .map_err(database_error)
}
