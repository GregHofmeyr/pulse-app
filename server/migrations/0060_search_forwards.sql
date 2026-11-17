-- Search finds a forward by the words it forwarded (#415). Until now a forward was indexed by its own content only:
-- the forwarder's comment, or the "↪ Forwarded a message" stand-in when they wrote none. Now its body is its comment
-- (nothing for the stand-in, which nobody said) then the original's text as it was forwarded
-- (`message_forwards.content`), while the original is live. Once the original is deleted or purged, its forwards'
-- bodies lose its words, as the forwards themselves do (they read as unavailable).
--
-- `search_message_update` (0011) is replaced so an update of a forward (a delete, mostly: forwards can't be edited)
-- writes the same body. Three new triggers: a forward's row arriving (just after its message, in the same
-- transaction), and its original being deleted or purged. The existing forwards are rewritten once at the end.
-- Triggers and FTS rows only: no table rebuild, so foreign keys being OFF while migrating (db.rs) changes nothing.
-- Rollback = restore the pre-deploy backup (or leave it: an older server keeps these triggers, which only ever write
-- the body described here).

DROP TRIGGER search_message_update;
CREATE TRIGGER search_message_update AFTER UPDATE OF id, channel_id, content, kind, deleted_at ON messages
BEGIN
    DELETE FROM message_search WHERE rowid = (SELECT doc FROM search_docs WHERE message_id = old.id);
    DELETE FROM search_docs WHERE message_id = old.id
        AND NOT (new.id = old.id AND new.kind = 'normal' AND new.deleted_at IS NULL);
    UPDATE search_docs SET channel_id = new.channel_id WHERE message_id = new.id;
    INSERT OR IGNORE INTO search_docs (message_id, channel_id)
        SELECT new.id, new.channel_id WHERE new.kind = 'normal' AND new.deleted_at IS NULL;
    INSERT INTO message_search (rowid, body)
        SELECT d.doc,
            replace(CASE WHEN EXISTS (SELECT 1 FROM message_forwards f WHERE f.message_id = new.id AND f.has_comment = 0)
                THEN '' ELSE new.content END, '[image]', '')
            || COALESCE(char(10) || (
                SELECT group_concat(l.caption, char(10)) FROM quote_lines l
                WHERE l.message_id = new.id AND l.caption IS NOT NULL), '')
            || COALESCE(char(10) || (
                SELECT f.content FROM message_forwards f JOIN messages o ON o.id = f.source_id AND o.deleted_at IS NULL
                WHERE f.message_id = new.id AND f.content != ''), '')
        FROM search_docs d WHERE d.message_id = new.id;
END;

-- A forward's row lands just after its message (indexed by its own content so far): write its body again.
CREATE TRIGGER search_forward_insert AFTER INSERT ON message_forwards
BEGIN
    DELETE FROM message_search WHERE rowid = (SELECT doc FROM search_docs WHERE message_id = new.message_id);
    INSERT INTO message_search (rowid, body)
        SELECT d.doc,
            (CASE WHEN new.has_comment = 0 THEN '' ELSE replace(m.content, '[image]', '') END)
            || COALESCE(char(10) || (
                SELECT new.content FROM messages o
                WHERE o.id = new.source_id AND o.deleted_at IS NULL AND new.content != ''), '')
        FROM search_docs d JOIN messages m ON m.id = d.message_id WHERE d.message_id = new.message_id;
END;

-- An original deleted: its forwards keep only their comments in the index.
CREATE TRIGGER search_forward_source_deleted AFTER UPDATE OF deleted_at ON messages
WHEN new.deleted_at IS NOT NULL AND old.deleted_at IS NULL
BEGIN
    DELETE FROM message_search WHERE rowid IN (
        SELECT d.doc FROM search_docs d JOIN message_forwards f ON f.message_id = d.message_id
        WHERE f.source_id = new.id);
    INSERT INTO message_search (rowid, body)
        SELECT d.doc, CASE WHEN f.has_comment = 0 THEN '' ELSE replace(m.content, '[image]', '') END
        FROM message_forwards f
        JOIN search_docs d ON d.message_id = f.message_id
        JOIN messages m ON m.id = f.message_id
        WHERE f.source_id = new.id;
END;

-- An original purged (with its chat): the same.
CREATE TRIGGER search_forward_source_purged AFTER DELETE ON messages
BEGIN
    DELETE FROM message_search WHERE rowid IN (
        SELECT d.doc FROM search_docs d JOIN message_forwards f ON f.message_id = d.message_id
        WHERE f.source_id = old.id);
    INSERT INTO message_search (rowid, body)
        SELECT d.doc, CASE WHEN f.has_comment = 0 THEN '' ELSE replace(m.content, '[image]', '') END
        FROM message_forwards f
        JOIN search_docs d ON d.message_id = f.message_id
        JOIN messages m ON m.id = f.message_id
        WHERE f.source_id = old.id;
END;

-- The forwards already here, once.
DELETE FROM message_search WHERE rowid IN (
    SELECT d.doc FROM search_docs d JOIN message_forwards f ON f.message_id = d.message_id);
INSERT INTO message_search (rowid, body)
    SELECT d.doc,
        (CASE WHEN f.has_comment = 0 THEN '' ELSE replace(m.content, '[image]', '') END)
        || COALESCE(char(10) || (
            SELECT f.content FROM messages o
            WHERE o.id = f.source_id AND o.deleted_at IS NULL AND f.content != ''), '')
    FROM message_forwards f
    JOIN search_docs d ON d.message_id = f.message_id
    JOIN messages m ON m.id = f.message_id;
