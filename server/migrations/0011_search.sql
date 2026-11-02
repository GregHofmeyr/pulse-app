-- Message search (spec 2026-11-02-message-search-design): an FTS5 index of message text, kept in
-- sync by the triggers below, and filled from the messages already here.
--
-- A message is in the index while it is a normal message and not deleted: system lines and
-- deleted messages never are, whatever text they still hold. Its body is its content, then the
-- captions of its quote's image lines (they live in `quote_lines`, not in the content). The
-- "[image]" a quote's picture line reads as in its content is left out: nobody said it.
--
-- Not an external-content table over `messages`: the body joins two tables, and a contentful
-- table can drop a row by rowid without being handed the old text back. The FTS rowid is
-- `search_docs.doc`, not `messages.rowid`: `messages` has a TEXT primary key, so its implicit
-- rowid may change under VACUUM (and in a `VACUUM INTO` backup), while an INTEGER PRIMARY KEY
-- doesn't. Docs are numbered in posting order (the build below goes by message id, a new message
-- gets the next one, and an edit keeps its doc), so a search's newest-first order is doc order,
-- which can be walked backwards and stopped after a page: through the FTS rowid for a whole
-- server, through `search_docs_channel` for one channel (a DM, or `in:`).
-- No foreign key on `message_id`: the triggers own these rows, and a cascade running before
-- them would hide the doc they need to find the FTS row.
CREATE TABLE search_docs (
    doc        INTEGER PRIMARY KEY,
    message_id TEXT NOT NULL UNIQUE,
    channel_id TEXT NOT NULL
) STRICT;
CREATE INDEX search_docs_channel ON search_docs(channel_id, doc);

CREATE VIRTUAL TABLE message_search USING fts5(
    body,
    tokenize = 'unicode61 remove_diacritics 2',
    prefix = '2 3'
);

CREATE TRIGGER search_message_insert AFTER INSERT ON messages
WHEN new.kind = 'normal' AND new.deleted_at IS NULL
BEGIN
    INSERT INTO search_docs (message_id, channel_id) VALUES (new.id, new.channel_id);
    -- A new message has no quote lines yet (they reference it), so its body is its content.
    INSERT INTO message_search (rowid, body)
        SELECT doc, replace(new.content, '[image]', '') FROM search_docs WHERE message_id = new.id;
END;

-- An edit, a delete or anything else that changes what is searchable: drop the old entry, and
-- add the new one if the message is still searchable, under the doc it had.
CREATE TRIGGER search_message_update AFTER UPDATE OF id, channel_id, content, kind, deleted_at ON messages
BEGIN
    DELETE FROM message_search WHERE rowid = (SELECT doc FROM search_docs WHERE message_id = old.id);
    DELETE FROM search_docs WHERE message_id = old.id
        AND NOT (new.id = old.id AND new.kind = 'normal' AND new.deleted_at IS NULL);
    UPDATE search_docs SET channel_id = new.channel_id WHERE message_id = new.id;
    INSERT OR IGNORE INTO search_docs (message_id, channel_id)
        SELECT new.id, new.channel_id WHERE new.kind = 'normal' AND new.deleted_at IS NULL;
    INSERT INTO message_search (rowid, body)
        SELECT d.doc, replace(new.content, '[image]', '') || COALESCE(char(10) || (
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
WHEN new.caption IS NOT NULL
BEGIN
    DELETE FROM message_search WHERE rowid = (SELECT doc FROM search_docs WHERE message_id = new.message_id);
    INSERT INTO message_search (rowid, body)
        SELECT d.doc, replace(m.content, '[image]', '') || COALESCE(char(10) || (
            SELECT group_concat(l.caption, char(10)) FROM quote_lines l
            WHERE l.message_id = m.id AND l.caption IS NOT NULL), '')
        FROM search_docs d JOIN messages m ON m.id = d.message_id WHERE d.message_id = new.message_id;
END;

-- Only when a caption really changed (a quote edit rewrites every kept line).
CREATE TRIGGER search_line_update AFTER UPDATE OF message_id, caption ON quote_lines
WHEN old.caption IS NOT new.caption OR old.message_id != new.message_id
BEGIN
    DELETE FROM message_search WHERE rowid IN (
        SELECT doc FROM search_docs WHERE message_id IN (old.message_id, new.message_id));
    INSERT INTO message_search (rowid, body)
        SELECT d.doc, replace(m.content, '[image]', '') || COALESCE(char(10) || (
            SELECT group_concat(l.caption, char(10)) FROM quote_lines l
            WHERE l.message_id = m.id AND l.caption IS NOT NULL), '')
        FROM search_docs d JOIN messages m ON m.id = d.message_id
        WHERE d.message_id IN (old.message_id, new.message_id);
END;

CREATE TRIGGER search_line_delete AFTER DELETE ON quote_lines
WHEN old.caption IS NOT NULL
BEGIN
    DELETE FROM message_search WHERE rowid = (SELECT doc FROM search_docs WHERE message_id = old.message_id);
    INSERT INTO message_search (rowid, body)
        SELECT d.doc, replace(m.content, '[image]', '') || COALESCE(char(10) || (
            SELECT group_concat(l.caption, char(10)) FROM quote_lines l
            WHERE l.message_id = m.id AND l.caption IS NOT NULL), '')
        FROM search_docs d JOIN messages m ON m.id = d.message_id WHERE d.message_id = old.message_id;
END;

-- The one-off build for the messages already here, in one pass each.
INSERT INTO search_docs (message_id, channel_id)
    SELECT id, channel_id FROM messages WHERE kind = 'normal' AND deleted_at IS NULL ORDER BY id;
INSERT INTO message_search (rowid, body)
    SELECT d.doc, replace(m.content, '[image]', '') || COALESCE(char(10) || (
        SELECT group_concat(l.caption, char(10)) FROM quote_lines l
        WHERE l.message_id = m.id AND l.caption IS NOT NULL), '')
    FROM search_docs d JOIN messages m ON m.id = d.message_id;
-- Merge the build's segments into one, so the first searches don't pay for it.
INSERT INTO message_search (message_search) VALUES ('optimize');
