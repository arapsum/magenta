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
