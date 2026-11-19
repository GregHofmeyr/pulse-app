-- Block a person (#474). A new table and a nullable column (no rebuilds), so foreign keys being OFF while migrating
-- (db.rs) changes nothing, and an older server ignores both.
-- Rollback = restore the pre-deploy backup (or leave them: an older server ignores both, so blocks stop applying and
-- withheld DMs would show to the person who had blocked their sender).

-- The people someone has blocked, and how they show in shared servers ('line': a "Blocked message" line, 'hide':
-- nothing of them). Yours only; they're never told. Goes with either person.
CREATE TABLE blocks (
    blocker_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    target_id  TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    mode       TEXT NOT NULL CHECK (mode IN ('line', 'hide')),
    created_at TEXT NOT NULL,
    PRIMARY KEY (blocker_id, target_id)
) STRICT;
CREATE INDEX blocks_by_target ON blocks (target_id);

-- A DM message sent to someone who had blocked its sender: it looks sent to its sender and never reaches the person
-- named here, not even after an unblock. NULL for every other message.
ALTER TABLE messages ADD COLUMN withheld_from TEXT;
CREATE INDEX messages_withheld ON messages (withheld_from) WHERE withheld_from IS NOT NULL;
