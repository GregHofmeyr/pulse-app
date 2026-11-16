-- Custom status (#152): a short line of text and an optional emoji beside your name, clearing on a timer or never;
-- and the last few you set, for picking again. Three ADD COLUMNs and a new table, no rebuilds, so foreign keys being
-- OFF while migrating (db.rs) changes nothing.
-- Rollback = restore the pre-deploy backup (or ignore the columns and table: older servers never read them).

-- The text, sanitised (trimmed, whitespace collapsed, no control or bidi characters, at most 128 characters). NULL =
-- no custom status; '' = an emoji alone.
ALTER TABLE users ADD COLUMN custom_text TEXT;
-- A standard emoji (canonical Unicode) or `custom:<id>`, as a reaction key; NULL = none.
ALTER TABLE users ADD COLUMN custom_emoji TEXT;
-- When it clears (RFC 3339 UTC, milliseconds); NULL = never. A run-out one counts as none; the status sweep clears it.
ALTER TABLE users ADD COLUMN custom_until TEXT;

-- Each person's recent custom statuses (the newest few are kept), newest `used_at` first. `emoji` is '' for none, so
-- the primary key treats "no emoji" as one value.
CREATE TABLE custom_status_recent (
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    text    TEXT NOT NULL,
    emoji   TEXT NOT NULL DEFAULT '',
    used_at TEXT NOT NULL,
    PRIMARY KEY (user_id, text, emoji)
) STRICT;
