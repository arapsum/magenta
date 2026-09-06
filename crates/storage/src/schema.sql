CREATE TABLE conversations (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    title TEXT NOT NULL,
    generation TEXT NOT NULL,
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

PRAGMA user_version = 2;
