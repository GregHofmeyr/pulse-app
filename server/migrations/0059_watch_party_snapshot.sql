-- Watch parties survive a server restart (#409). A party is still held in memory (`watch::WatchState`); this table
-- only keeps a snapshot of each one, written on every change and removed when it ends. On boot, once voice presence
-- is back from LiveKit, a snapshot saved within the last few minutes is restored with only the viewers still in the
-- call (none left: it's dropped). Older ones are thrown away. A new table, no rebuild.
-- Rollback = restore the pre-deploy backup (or ignore the table: older servers never read it).

CREATE TABLE watch_party_snapshots (
    channel_id  TEXT PRIMARY KEY REFERENCES channels(id) ON DELETE CASCADE,
    -- the party as the protocol's `WatchParty`, in JSON
    party       TEXT NOT NULL,
    -- when it was written (Unix milliseconds, server clock)
    saved_at_ms INTEGER NOT NULL
) STRICT;
