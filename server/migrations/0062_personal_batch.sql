-- Per-person Do Not Disturb (#310). New tables only (no rebuilds), so foreign keys being OFF while migrating (db.rs)
-- changes nothing, and older servers never read them.
-- Rollback = restore the pre-deploy backup (or leave the tables: nothing else reads them).

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
