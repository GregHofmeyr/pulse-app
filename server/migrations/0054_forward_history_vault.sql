-- Edit history (#301), forwarded messages (#302) and the personal message vault (#315). New tables only (no rebuilds),
-- so foreign keys being OFF while migrating (db.rs) changes nothing.
-- Rollback = restore the pre-deploy backup (or drop the three tables: older servers never read them; a forward then
-- reads as its stand-in text or comment, and edit history and saved messages are lost, nothing else).

-- The versions a message had before its edits (#301), one row per edit: the text it had, and when that text was written
-- (the message's `created_at`, or the edit before). Read by anyone who can see the message. A delete takes them with it
-- (in its transaction; a purge by the cascade). Each message keeps its newest few (the cap is in the server).
CREATE TABLE message_edits (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    message_id  TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    content     TEXT NOT NULL,
    written_at  TEXT NOT NULL,
    replaced_at TEXT NOT NULL
) STRICT;

-- A message's versions, newest first.
CREATE INDEX message_edits_message ON message_edits(message_id, id);

-- A forwarded message (#302): `message_id` is the forward (a normal message in the chat it was forwarded to, whose own
-- content is the forwarder's comment, or a stand-in when they wrote none: `has_comment`); `source_id` is the original,
-- never itself a forward (forwarding a forward points at its original). No foreign key on `source_id`: an original
-- purged with its chat leaves the forward reading as unavailable, as a deleted one does. `content` and `gif` are the
-- original's at the time it was forwarded (a later edit doesn't change what was forwarded); its files are read from the
-- original, by reference, while it is there. Goes with the forward.
CREATE TABLE message_forwards (
    message_id        TEXT PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
    source_id         TEXT NOT NULL,
    source_channel_id TEXT NOT NULL,
    source_author_id  TEXT,
    source_created_at TEXT NOT NULL,
    content           TEXT NOT NULL,
    gif               TEXT,
    has_comment       INTEGER NOT NULL DEFAULT 0
) STRICT;

-- An original's forwards: told when it is deleted.
CREATE INDEX message_forwards_source ON message_forwards(source_id);

-- Saved messages (#315): private bookmarks, one per person and message, with an optional note. No foreign key on
-- `message_id`: one whose message is deleted, purged or out of reach is listed as unavailable (never its content) until
-- removed. Goes with its person.
CREATE TABLE message_bookmarks (
    id         TEXT PRIMARY KEY,
    user_id    TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    message_id TEXT NOT NULL,
    note       TEXT,
    created_at TEXT NOT NULL,
    UNIQUE (user_id, message_id)
) STRICT;

-- A person's list, newest first.
CREATE INDEX message_bookmarks_user ON message_bookmarks(user_id, created_at);
