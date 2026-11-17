-- Poll edits (#265) and DM categories and pins (#305). A nullable column and new tables only (no rebuilds), so
-- foreign keys being OFF while migrating (db.rs) changes nothing.
-- Rollback = restore the pre-deploy backup (or leave it: older servers never read it).

-- When a poll's creator last changed its question or options (#265); the card says "(edited)".
ALTER TABLE polls ADD COLUMN edited_at TEXT;

-- DM categories and pinned conversations (#305). New tables only. Rollback = drop them: older servers never read them,
-- and everyone's conversation list goes back to newest first.

-- Your own headings in your conversation list, only you see them. `position` is their order (new ones last).
-- Ids are minted by the server and unique on their own; `user_id` keeps every read and write to its owner.
CREATE TABLE dm_categories (
    id         TEXT PRIMARY KEY,
    user_id    TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name       TEXT NOT NULL,
    position   INTEGER NOT NULL,
    collapsed  INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL
) STRICT;

CREATE INDEX dm_categories_user ON dm_categories(user_id, position);

-- Where one of your conversations sits: pinned to the top and/or in one of your categories (a row with neither is
-- deleted). Deleting a category puts its conversations back in the main list. Goes with its person or its chat.
CREATE TABLE dm_placements (
    user_id     TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    channel_id  TEXT NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
    pinned      INTEGER NOT NULL DEFAULT 0,
    category_id TEXT REFERENCES dm_categories(id) ON DELETE SET NULL,
    PRIMARY KEY (user_id, channel_id)
) STRICT;

CREATE INDEX dm_placements_category ON dm_placements(category_id);
