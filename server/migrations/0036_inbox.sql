-- Inbox (#306): your mentions and replies across every server, DM and group. Items are the existing
-- `mentions` rows (a reply already pings whoever it answers); this adds what tells the two apart, an
-- index to page them by, and the two small read tables. One added column, an UPDATE of that column,
-- an index swap and two new tables: nothing is rebuilt or dropped but an index. The guard makes the
-- migration fail, rolled back, if it is ever run with foreign keys on (`db::connect` runs migrations
-- with them OFF).
-- Rollback = restore the pre-deploy backup (or drop the two tables and the column: the send path of
-- older servers never reads them).
CREATE TEMP TABLE migration_guard (ok INTEGER NOT NULL CHECK (ok = 0));
INSERT INTO migration_guard SELECT foreign_keys FROM pragma_foreign_keys;
DROP TABLE migration_guard;

-- 1 = the message typed `@you`; 0 = only the ping of a reply to your message. Existing rows: a reply to
-- that person's message whose text doesn't hold `@<their username>` was only the reply's ping (old rows
-- only: a username that is a prefix of the typed one reads as typed, which is harmless).
ALTER TABLE mentions ADD COLUMN explicit INTEGER NOT NULL DEFAULT 1;
UPDATE mentions SET explicit = 0
WHERE EXISTS (
    SELECT 1 FROM messages m
    JOIN messages p ON p.id = m.reply_to_id
    JOIN users u ON u.id = mentions.user_id
    WHERE m.id = mentions.message_id AND p.author_id = mentions.user_id
      AND instr(lower(m.content), '@' || lower(u.username)) = 0
);

-- The inbox pages a person's mentions newest first by message id; this replaces `mentions_user
-- (user_id)`, which is a prefix of it.
CREATE INDEX mentions_user_message ON mentions(user_id, message_id);
DROP INDEX mentions_user;

-- Items opened from the inbox (read on every device). Pruned after 30 days, when the item has left the
-- inbox anyway.
CREATE TABLE inbox_reads (
    user_id    TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    message_id TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    PRIMARY KEY (user_id, message_id)
) STRICT;

-- Mark all read: every item up to `upto` (a message id) is read.
CREATE TABLE inbox_cleared (
    user_id TEXT PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    upto    TEXT NOT NULL
) STRICT;
