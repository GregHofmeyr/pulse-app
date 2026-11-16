-- Steam account linking and live achievements (#379). Additive: new tables only (no rebuilds, so foreign keys being
-- OFF while migrating changes nothing). Nobody is linked yet, so nothing is polled until someone links. Rollback =
-- leave them (older servers never read them) or drop the six tables.

-- One Steam account per person, and one person per Steam account.
CREATE TABLE steam_links (
    user_id            TEXT NOT NULL PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    steam_id           TEXT NOT NULL UNIQUE,
    persona            TEXT,
    -- "Share Steam achievements" (default on): off = never polled, nothing posted.
    share_achievements INTEGER NOT NULL DEFAULT 1,
    linked_at          TEXT NOT NULL,
    -- 'pending' (waiting for the backfill worker), 'private' (Steam said game details are private), 'done'
    backfill           TEXT NOT NULL DEFAULT 'pending',
    -- When Steam last refused to show this person's game details (cleared by the next good answer).
    private_since      TEXT,
    -- Average completion (0..100) over games with at least one unlock; NULL until there's one.
    completion_rate    REAL,
    perfect_games      INTEGER NOT NULL DEFAULT 0
) STRICT;

-- What the server last saw of one person's achievements in one game: the unlocked API names (a JSON array).
CREATE TABLE steam_achievement_state (
    user_id    TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    app_id     INTEGER NOT NULL,
    unlocked   TEXT NOT NULL,
    total      INTEGER NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (user_id, app_id)
) STRICT;

-- A game's achievement schema and global rarity, refreshed after a day (JSON as the server reads it).
CREATE TABLE steam_app_cache (
    app_id     INTEGER NOT NULL PRIMARY KEY,
    name       TEXT,
    schema     TEXT NOT NULL,
    rarity     TEXT NOT NULL,
    fetched_at TEXT NOT NULL
) STRICT;

-- Every unlock announced live and every 100 %, with its global rarity: the yearly awards' material. Kept after an
-- unlink (history); goes with the person's account.
CREATE TABLE steam_unlocks (
    id             INTEGER PRIMARY KEY,
    user_id        TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    app_id         INTEGER NOT NULL,
    -- an achievement's API name, or NULL for a 100 % completion
    api_name       TEXT,
    name           TEXT NOT NULL,
    global_percent REAL,
    unlocked_at    TEXT NOT NULL
) STRICT;
CREATE INDEX steam_unlocks_user ON steam_unlocks (user_id, unlocked_at);

-- Media an achievement card shows (icons, game art): kept from the media sweep.
CREATE TABLE steam_card_media (
    hash TEXT NOT NULL PRIMARY KEY
) STRICT;

-- Each server's Achievements channel (owner's choice); no row = no posts.
CREATE TABLE server_achievement_channels (
    server_id  TEXT NOT NULL PRIMARY KEY REFERENCES servers(id) ON DELETE CASCADE,
    channel_id TEXT NOT NULL REFERENCES channels(id) ON DELETE CASCADE
) STRICT;
