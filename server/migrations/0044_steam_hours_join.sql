-- Steam hours (#383) and one-click Join (#381). Additive: new columns on steam_links only (no rebuilds, so foreign keys
-- being OFF while migrating changes nothing). Both switches default on; nothing is read until the poller next runs.
-- Rollback = leave them (older servers never read them).

-- "Share Steam hours" (default on): off = their recent play time is never read or shown.
ALTER TABLE steam_links ADD COLUMN share_hours INTEGER NOT NULL DEFAULT 1;
-- Minutes played in Steam games over the last two weeks (Steam's `playtime_2weeks`, summed); NULL until first read.
ALTER TABLE steam_links ADD COLUMN recent_minutes INTEGER;
-- When `recent_minutes` was last read.
ALTER TABLE steam_links ADD COLUMN recent_at TEXT;
-- "Share Join" (default on): off = their lobby or game server is never read, so nobody is offered Join.
ALTER TABLE steam_links ADD COLUMN share_join INTEGER NOT NULL DEFAULT 1;
