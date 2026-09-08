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
    omitted_context_messages INTEGER NOT NULL DEFAULT 0 CHECK (omitted_context_messages >= 0),
    created_at INTEGER NOT NULL,
    UNIQUE (conversation_id, sequence)
);

CREATE UNIQUE INDEX one_stream_per_conversation
    ON messages(conversation_id)
    WHERE status = 'streaming';

CREATE INDEX conversation_recency ON conversations(updated_at DESC, id DESC);

CREATE TABLE projects (
    root BLOB PRIMARY KEY,
    name TEXT NOT NULL,
    added_at INTEGER NOT NULL,
    last_opened_at INTEGER NOT NULL
);

CREATE INDEX project_recency ON projects(last_opened_at DESC, name);

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

-- Keep search indexes external to the source tables so normal writes remain the
-- single source of truth. Triggers below maintain the indexes transactionally.
CREATE VIRTUAL TABLE conversation_fts USING fts5(
    title,
    content = 'conversations',
    content_rowid = 'id',
    tokenize = 'unicode61 remove_diacritics 2'
);

CREATE VIRTUAL TABLE message_fts USING fts5(
    content,
    content = 'messages',
    content_rowid = 'id',
    tokenize = 'unicode61 remove_diacritics 2'
);

CREATE TRIGGER conversation_fts_insert AFTER INSERT ON conversations BEGIN
    INSERT INTO conversation_fts(rowid, title) VALUES (new.id, new.title);
END;

CREATE TRIGGER conversation_fts_delete AFTER DELETE ON conversations BEGIN
    INSERT INTO conversation_fts(conversation_fts, rowid, title)
    VALUES ('delete', old.id, old.title);
END;

CREATE TRIGGER conversation_fts_update AFTER UPDATE OF title ON conversations BEGIN
    INSERT INTO conversation_fts(conversation_fts, rowid, title)
    VALUES ('delete', old.id, old.title);
    INSERT INTO conversation_fts(rowid, title) VALUES (new.id, new.title);
END;

CREATE TRIGGER message_fts_insert AFTER INSERT ON messages BEGIN
    INSERT INTO message_fts(rowid, content) VALUES (new.id, new.content);
END;

CREATE TRIGGER message_fts_delete AFTER DELETE ON messages BEGIN
    INSERT INTO message_fts(message_fts, rowid, content)
    VALUES ('delete', old.id, old.content);
END;

CREATE TRIGGER message_fts_update AFTER UPDATE OF content ON messages BEGIN
    INSERT INTO message_fts(message_fts, rowid, content)
    VALUES ('delete', old.id, old.content);
    INSERT INTO message_fts(rowid, content) VALUES (new.id, new.content);
END;

PRAGMA user_version = 6;
