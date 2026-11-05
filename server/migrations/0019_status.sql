-- Status (#45): Online / Do Not Disturb / Invisible, and appearing offline to chosen people.
-- The unused `users.status` from 0001 (CHECK online/dnd) stays as it is: widening its CHECK would mean rebuilding
-- `users`, which every other table references. `presence` replaces it.
ALTER TABLE users ADD COLUMN presence TEXT NOT NULL DEFAULT 'online'
    CHECK (presence IN ('online', 'dnd', 'invisible'));
-- When a timed DND ends (RFC 3339 UTC); NULL = until turned off. An expired one counts as online.
ALTER TABLE users ADD COLUMN dnd_until TEXT;
-- While invisible: the moment you stopped looking online, shown to others as your last seen.
ALTER TABLE users ADD COLUMN invisible_at TEXT;

-- The people `owner_id` appears offline to. `hidden_at` is the last seen `target_id` sees (frozen). `active = 0`:
-- taken off the list while the owner is Invisible; it still freezes that person's view until the owner is visible
-- again, then the row goes (so their last seen never jumps).
CREATE TABLE presence_hidden (
    owner_id  TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    target_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    hidden_at TEXT NOT NULL,
    active    INTEGER NOT NULL DEFAULT 1 CHECK (active IN (0, 1)),
    PRIMARY KEY (owner_id, target_id)
) STRICT;
