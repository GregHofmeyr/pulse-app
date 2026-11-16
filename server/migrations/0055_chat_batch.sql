-- Poll edits (#265). Adds a nullable column only (no rebuilds), so foreign keys being OFF while migrating (db.rs)
-- changes nothing.
-- Rollback = restore the pre-deploy backup (or leave it: older servers never read it).

-- When a poll's creator last changed its question or options (#265); the card says "(edited)".
ALTER TABLE polls ADD COLUMN edited_at TEXT;
