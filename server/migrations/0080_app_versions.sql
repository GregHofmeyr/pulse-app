-- Who's on which app version (#534): each person's last update check and last connect, read by `pulse-app-server
-- versions` (`just versions`). One row per person, overwritten each time. A new table only, so foreign keys being OFF
-- while migrating (db.rs) changes nothing, and an older server ignores it.
-- Rollback = restore the pre-deploy backup (or leave it: an older server never reads or writes it).

-- `check_*`: the last GET /updates/latest.json. `check_version` is the app's own version from its `X-Pulse-Version`
-- header (NULL when it sent none); `check_result` what it was told: 'offered' (the manifest is newer), 'up_to_date',
-- 'no_manifest' (nothing published: 404) or 'unknown' (no version sent to compare).
-- `connect_*`: the last gateway connect; `connect_version` from the same header on the upgrade, NULL from apps before
-- it (they still show up, with an unknown version).
CREATE TABLE app_versions (
    user_id         TEXT PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    check_version   TEXT,
    check_result    TEXT CHECK (check_result IN ('offered', 'up_to_date', 'no_manifest', 'unknown')),
    checked_at      TEXT,
    connect_version TEXT,
    connected_at    TEXT
) STRICT;
