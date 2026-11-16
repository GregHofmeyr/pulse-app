-- Archived channels (#303): the server owner can archive a text channel, making it read-only (nobody posts, reacts or
-- edits there; the server refuses) and tucking it into a collapsed "Archived" group at the bottom of the channel list.
-- Unlike a delete (#112, `deleted_at`) it stays visible, readable and searchable, and it can be unarchived. One ADD
-- COLUMN, no rebuild, so foreign keys being OFF while migrating (db.rs) changes nothing.
-- Rollback = restore the pre-deploy backup (or ignore the column: older servers never read it, and archived channels
-- then simply take messages again).

-- When it was archived (RFC 3339 UTC, milliseconds); NULL = not archived.
ALTER TABLE channels ADD COLUMN archived_at TEXT;
