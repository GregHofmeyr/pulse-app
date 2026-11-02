-- Message search (spec 2026-11-02-message-search-design): an FTS5 index of message text, kept in
-- sync by the triggers below, and filled from the messages already here.
--
-- A message is in the index while it is a normal message and not deleted: system lines and
-- deleted messages never are, whatever text they still hold. Its body is its content, then the
-- captions of its quote's image lines (they live in `quote_lines`, not in the content).
--
-- Not an external-content table over `messages`: the body joins two tables, and a contentful
-- table can drop a row by rowid without being handed the old text back. The FTS rowid is
-- `search_docs.doc`, not `messages.rowid`: `messages` has a TEXT primary key, so its implicit
-- rowid may change under VACUUM (and in a `VACUUM INTO` backup), while an INTEGER PRIMARY KEY
-- doesn't. No foreign key on `message_id`: the triggers own these rows, and a cascade running
-- before them would hide the doc they need to find the FTS row.
CREATE TABLE search_docs (
    doc        INTEGER PRIMARY KEY,
    message_id TEXT NOT NULL UNIQUE
) STRICT;

CREATE VIRTUAL TABLE message_search USING fts5(
    body,
    tokenize = 'unicode61 remove_diacritics 2',
    prefix = '2 3'
);

CREATE TRIGGER search_message_insert AFTER INSERT ON messages
WHEN new.kind = 'normal' AND new.deleted_at IS NULL
BEGIN
    INSERT INTO search_docs (message_id) VALUES (new.id);
    -- A new message has no quote lines yet (they reference it), so its body is its content.
    INSERT INTO message_search (rowid, body)
        SELECT doc, new.content FROM search_docs WHERE message_id = new.id;
END;

-- An edit, a delete or anything else that changes what is searchable: drop the old entry, and
-- add the new one if the message is still searchable.
CREATE TRIGGER search_message_update AFTER UPDATE OF id, content, kind, deleted_at ON messages
BEGIN
    DELETE FROM message_search WHERE rowid = (SELECT doc FROM search_docs WHERE message_id = old.id);
    DELETE FROM search_docs WHERE message_id = old.id;
    INSERT INTO search_docs (message_id)
        SELECT new.id WHERE new.kind = 'normal' AND new.deleted_at IS NULL;
    INSERT INTO message_search (rowid, body)
        SELECT d.doc, new.content || COALESCE(char(10) || (
            SELECT group_concat(l.caption, char(10)) FROM quote_lines l
            WHERE l.message_id = new.id AND l.caption IS NOT NULL), '')
        FROM search_docs d WHERE d.message_id = new.id;
END;

CREATE TRIGGER search_message_delete AFTER DELETE ON messages
BEGIN
    DELETE FROM message_search WHERE rowid = (SELECT doc FROM search_docs WHERE message_id = old.id);
    DELETE FROM search_docs WHERE message_id = old.id;
END;

-- A quote's lines change its captions: write its body again (a message without a doc, such as a
-- deleted quote whose lines are going, is left out).
CREATE TRIGGER search_line_insert AFTER INSERT ON quote_lines
BEGIN
    DELETE FROM message_search WHERE rowid = (SELECT doc FROM search_docs WHERE message_id = new.message_id);
    INSERT INTO message_search (rowid, body)
        SELECT d.doc, m.content || COALESCE(char(10) || (
            SELECT group_concat(l.caption, char(10)) FROM quote_lines l
            WHERE l.message_id = m.id AND l.caption IS NOT NULL), '')
        FROM search_docs d JOIN messages m ON m.id = d.message_id WHERE d.message_id = new.message_id;
END;

CREATE TRIGGER search_line_update AFTER UPDATE OF message_id, caption ON quote_lines
BEGIN
    DELETE FROM message_search WHERE rowid = (SELECT doc FROM search_docs WHERE message_id = old.message_id);
    INSERT INTO message_search (rowid, body)
        SELECT d.doc, m.content || COALESCE(char(10) || (
            SELECT group_concat(l.caption, char(10)) FROM quote_lines l
            WHERE l.message_id = m.id AND l.caption IS NOT NULL), '')
        FROM search_docs d JOIN messages m ON m.id = d.message_id WHERE d.message_id = old.message_id;
    DELETE FROM message_search WHERE rowid = (SELECT doc FROM search_docs WHERE message_id = new.message_id)
        AND new.message_id != old.message_id;
    INSERT INTO message_search (rowid, body)
        SELECT d.doc, m.content || COALESCE(char(10) || (
            SELECT group_concat(l.caption, char(10)) FROM quote_lines l
            WHERE l.message_id = m.id AND l.caption IS NOT NULL), '')
        FROM search_docs d JOIN messages m ON m.id = d.message_id
        WHERE d.message_id = new.message_id AND new.message_id != old.message_id;
END;

CREATE TRIGGER search_line_delete AFTER DELETE ON quote_lines
BEGIN
    DELETE FROM message_search WHERE rowid = (SELECT doc FROM search_docs WHERE message_id = old.message_id);
    INSERT INTO message_search (rowid, body)
        SELECT d.doc, m.content || COALESCE(char(10) || (
            SELECT group_concat(l.caption, char(10)) FROM quote_lines l
            WHERE l.message_id = m.id AND l.caption IS NOT NULL), '')
        FROM search_docs d JOIN messages m ON m.id = d.message_id WHERE d.message_id = old.message_id;
END;

-- The one-off build for the messages already here, in one pass each.
INSERT INTO search_docs (message_id)
    SELECT id FROM messages WHERE kind = 'normal' AND deleted_at IS NULL ORDER BY id;
INSERT INTO message_search (rowid, body)
    SELECT d.doc, m.content || COALESCE(char(10) || (
        SELECT group_concat(l.caption, char(10)) FROM quote_lines l
        WHERE l.message_id = m.id AND l.caption IS NOT NULL), '')
    FROM search_docs d JOIN messages m ON m.id = d.message_id;
-- Merge the build's segments into one, so the first searches don't pay for it.
INSERT INTO message_search (message_search) VALUES ('optimize');
