-- Pinned messages (#178): one row per pinned message, in its chat, with who pinned it and when it runs out
-- (NULL: Forever). A new table only, nothing rebuilt or copied. A pin goes with its message or its channel on a
-- hard delete (the cascades); a soft delete removes it in the delete's own transaction (`messages::routes::remove`).
-- A row past `expires_at` is never shown; the sweep (`pins::expire_overdue`) deletes it and tells the chat.
CREATE TABLE pins (
    message_id TEXT PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
    channel_id TEXT NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
    pinned_by  TEXT NOT NULL REFERENCES users(id),
    pinned_at  TEXT NOT NULL,
    expires_at TEXT
) STRICT;
CREATE INDEX pins_channel ON pins (channel_id, pinned_at);
CREATE INDEX pins_expiry ON pins (expires_at) WHERE expires_at IS NOT NULL;
