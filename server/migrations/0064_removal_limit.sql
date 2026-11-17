-- Repeat removals (#418): a removed person may rejoin a server, but the third time its owner removes the same person
-- within 48 hours they're blocked from rejoining until the owner lets them back (Server settings, Members). New tables
-- only (no rebuilds), so foreign keys being OFF while migrating (db.rs) changes nothing, and older servers never read
-- them.
-- Rollback = restore the pre-deploy backup (or leave them: an older server ignores them, and blocked people can then
-- simply rejoin).

-- Each removal by a server's owner (not a leave), and when (RFC 3339 UTC, milliseconds). Only the last 48 hours count;
-- older rows are pruned at the next removal of the same person. Goes with its server or the removed person; the
-- remover's id is kept for the record and cleared when they go.
CREATE TABLE server_removals (
    server_id  TEXT NOT NULL REFERENCES servers(id) ON DELETE CASCADE,
    user_id    TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    removed_by TEXT REFERENCES users(id) ON DELETE SET NULL,
    removed_at TEXT NOT NULL
) STRICT;
CREATE INDEX server_removals_by_person ON server_removals (server_id, user_id, removed_at);

-- People who can't rejoin a server (the third removal in 48 hours), until its owner allows them back, which deletes
-- the row (and their counted removals). No expiry: only the owner lifts it.
CREATE TABLE server_blocks (
    server_id  TEXT NOT NULL REFERENCES servers(id) ON DELETE CASCADE,
    user_id    TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    blocked_at TEXT NOT NULL,
    PRIMARY KEY (server_id, user_id)
) STRICT;
