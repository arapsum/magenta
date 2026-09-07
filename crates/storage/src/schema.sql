CREATE TABLE conversations (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    title TEXT NOT NULL,
    generation TEXT NOT NULL,
    mode TEXT NOT NULL DEFAULT 'chat' CHECK (mode IN ('chat', 'agent')),
    workspace_root BLOB,
    pinned INTEGER NOT NULL DEFAULT 0 CHECK (pinned IN (0, 1)),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE messages (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    conversation_id INTEGER NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    sequence INTEGER NOT NULL CHECK (sequence >= 0),
    role TEXT NOT NULL CHECK (role IN ('user', 'assistant')),
    content TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('complete', 'streaming', 'stopped', 'failed')),
    generation TEXT NOT NULL,
    outcome TEXT,
    created_at INTEGER NOT NULL,
    UNIQUE (conversation_id, sequence)
);

CREATE UNIQUE INDEX one_stream_per_conversation
    ON messages(conversation_id)
    WHERE status = 'streaming';

CREATE INDEX conversation_recency ON conversations(updated_at DESC, id DESC);

CREATE TABLE agent_runs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    conversation_id INTEGER NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    assistant_message_id INTEGER NOT NULL UNIQUE REFERENCES messages(id) ON DELETE CASCADE,
    status TEXT NOT NULL CHECK (status IN ('running', 'completed', 'stopped', 'failed')),
    started_at INTEGER NOT NULL,
    finished_at INTEGER
);

CREATE TABLE agent_activities (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id INTEGER NOT NULL REFERENCES agent_runs(id) ON DELETE CASCADE,
    sequence INTEGER NOT NULL CHECK (sequence >= 0),
    kind TEXT NOT NULL CHECK (kind IN ('tool-call', 'approval-requested', 'tool-result')),
    call_id TEXT NOT NULL,
    tool_name TEXT NOT NULL,
    status TEXT NOT NULL,
    summary TEXT NOT NULL,
    detail TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    UNIQUE (run_id, sequence)
);

CREATE INDEX agent_activity_order ON agent_activities(run_id, sequence);

CREATE TABLE attachments (
    message_id INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    name TEXT NOT NULL,
    source_path BLOB NOT NULL,
    mime_type TEXT NOT NULL,
    byte_size INTEGER NOT NULL CHECK (byte_size >= 0),
    managed INTEGER NOT NULL CHECK (managed IN (0, 1)),
    PRIMARY KEY (message_id, position)
);

PRAGMA user_version = 3;
