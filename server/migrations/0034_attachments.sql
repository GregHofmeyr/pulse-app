-- Attachments (#57): files sent with messages. A new table only (no rebuilds); the guard still
-- makes the migration fail, rolled back, if it is ever run with foreign keys on (`db::connect`
-- runs migrations with them OFF).
-- Rollback = restore the pre-deploy backup (or drop the table: a message with files keeps its
-- caption or its "📎 …" fallback text as `content`).
CREATE TEMP TABLE migration_guard (ok INTEGER NOT NULL CHECK (ok = 0));
INSERT INTO migration_guard SELECT foreign_keys FROM pragma_foreign_keys;
DROP TABLE migration_guard;

-- One row per file, from the moment its upload starts. `message_id` is NULL while it is pending
-- (uploaded or not, but not yet sent). `key` is server-assigned (`media::new_key()`, never the
-- file's name): the object is `attachments/<key>`, or `temp/<key>` for a temporary send
-- (`expires_at` set). SET NULL, not CASCADE, on both references: a row must outlive its message
-- or channel, or the sweeper would lose the key of an object that is stored.
CREATE TABLE attachments (
    id           TEXT PRIMARY KEY,
    key          TEXT NOT NULL UNIQUE,
    user_id      TEXT NOT NULL REFERENCES users(id),
    channel_id   TEXT REFERENCES channels(id) ON DELETE SET NULL,
    message_id   TEXT REFERENCES messages(id) ON DELETE SET NULL,
    position     INTEGER NOT NULL DEFAULT 0,
    name         TEXT NOT NULL,
    content_type TEXT NOT NULL,
    kind         TEXT NOT NULL CHECK (kind IN ('image', 'video', 'audio', 'file')),
    size         INTEGER NOT NULL,
    width        INTEGER,
    height       INTEGER,
    duration_ms  INTEGER,
    expires_at   TEXT,    -- temporary sends only
    upload_id    TEXT,    -- the multipart upload's id (temporary sends)
    purged_at    TEXT,    -- when an expired temporary file's object was deleted
    created_at   TEXT NOT NULL
) STRICT;
-- A message's files in order.
CREATE INDEX attachments_message ON attachments(message_id, position) WHERE message_id IS NOT NULL;
-- The sweep's look for uploads never sent.
CREATE INDEX attachments_pending ON attachments(created_at) WHERE message_id IS NULL;
-- One person's open uploads (the cap on starting more).
CREATE INDEX attachments_user_pending ON attachments(user_id) WHERE message_id IS NULL;
-- Live temporary files: the 20 GB cap and the expiry sweep.
CREATE INDEX attachments_live_temp ON attachments(expires_at) WHERE expires_at IS NOT NULL AND purged_at IS NULL;
