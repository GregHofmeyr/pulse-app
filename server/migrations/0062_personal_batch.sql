-- Per-person Do Not Disturb (#310), saved inbox filters (#307), personal channel orders (#304) and view-once files
-- (#311). New tables and one new column with a constant default only (no rebuilds), so foreign keys being OFF while
-- migrating (db.rs) changes nothing, and older servers never read them.
-- Rollback = restore the pre-deploy backup (or leave them: nothing else reads them). NB an older server would serve
-- view-once files to everyone like any other file, so don't roll back past this while any are in use.

-- People whose messages don't alert you (no sound, no desktop notification), everywhere or only in DMs and groups,
-- until `until` (NULL: until turned back off). Yours only; they're never told. A row past its `until` reads as none.
CREATE TABLE person_dnd (
    user_id    TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    target_id  TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    dms_only   INTEGER NOT NULL DEFAULT 0 CHECK (dms_only IN (0, 1)),
    until      TEXT,
    created_at TEXT NOT NULL,
    PRIMARY KEY (user_id, target_id)
) STRICT;

-- Saved inbox filters (#307): the inbox narrowed to some people and/or chats (JSON arrays of ids; either empty: any),
-- optionally only unread. In your order (`position`); the name is unique per person ignoring case (checked by the
-- server). Yours only; the whole list is replaced on every save.
CREATE TABLE inbox_filters (
    user_id     TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    position    INTEGER NOT NULL,
    name        TEXT NOT NULL,
    people      TEXT NOT NULL DEFAULT '[]',
    channels    TEXT NOT NULL DEFAULT '[]',
    unread_only INTEGER NOT NULL DEFAULT 0 CHECK (unread_only IN (0, 1)),
    PRIMARY KEY (user_id, position)
) STRICT;

-- Your own order of a server's channel list (#304), drawn over the server's layout for you only. `body` is JSON:
-- {"uncategorised": [channel ids], "categories": [{"id": category id, "channels": [channel ids]}]}, cleaned against the
-- server when saved. Goes with its person or its server; one you left is kept but not read.
CREATE TABLE channel_orders (
    user_id    TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    server_id  TEXT NOT NULL REFERENCES servers(id) ON DELETE CASCADE,
    body       TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (user_id, server_id)
) STRICT;

-- View-once files (#311): each recipient can open them once; the server signs no other link to them for anyone but
-- their sender.
ALTER TABLE attachments ADD COLUMN view_once INTEGER NOT NULL DEFAULT 0 CHECK (view_once IN (0, 1));

-- Who opened a message's view-once files, and when (first open only). Goes with the message or the person.
CREATE TABLE view_once_views (
    message_id TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    user_id    TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    viewed_at  TEXT NOT NULL,
    PRIMARY KEY (message_id, user_id)
) STRICT;
