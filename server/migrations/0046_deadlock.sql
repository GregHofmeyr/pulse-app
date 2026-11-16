-- Deadlock (#390): post-match cards, rank badges and weekly rank movers, squad stats, patch notes; all from
-- deadlock-api.com, keyed by the Steam account people already linked (#379). Additive: new columns on steam_links and
-- new tables only (no rebuilds, so foreign keys being OFF while migrating changes nothing). Nothing is read until the
-- poller next runs, and nothing is posted until a server's owner picks a Deadlock channel. Rollback = leave them
-- (older servers never read them) or drop the seven tables.

-- Each person's three switches (default on): post-match cards, the rank badge (and rank movers), squad stats.
ALTER TABLE steam_links ADD COLUMN deadlock_matches INTEGER NOT NULL DEFAULT 1;
ALTER TABLE steam_links ADD COLUMN deadlock_rank INTEGER NOT NULL DEFAULT 1;
ALTER TABLE steam_links ADD COLUMN deadlock_squad INTEGER NOT NULL DEFAULT 1;

-- What the server last saw of a linked person in Deadlock. `account_id` is the SteamID3 the row was made for: a
-- different linked account starts over. `last_match_id` NULL = no baseline yet (the first read posts nothing).
CREATE TABLE deadlock_players (
    user_id       TEXT NOT NULL PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    account_id    INTEGER NOT NULL,
    last_match_id INTEGER,
    -- rank badge (tier * 10 + subrank) from the last rank read; NULL = unknown, 0 = unranked
    badge         INTEGER,
    rank_at       TEXT,
    -- the badge when the last weekly rank movers ran: this week's movement is from here
    week_badge    INTEGER
) STRICT;

-- The matches seen for each person (their line on the card, and "last match" on their profile).
CREATE TABLE deadlock_matches (
    user_id     TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    match_id    INTEGER NOT NULL,
    hero_id     INTEGER NOT NULL,
    kills       INTEGER NOT NULL,
    deaths      INTEGER NOT NULL,
    assists     INTEGER NOT NULL,
    won         INTEGER NOT NULL,
    duration_s  INTEGER NOT NULL,
    rank_delta  INTEGER,
    badge_after INTEGER,
    started_at  TEXT NOT NULL,
    PRIMARY KEY (user_id, match_id)
) STRICT;
CREATE INDEX deadlock_matches_recent ON deadlock_matches (user_id, started_at);

-- The card posted for a match in a channel, so friends in the same match share one card.
CREATE TABLE deadlock_match_posts (
    match_id   INTEGER NOT NULL,
    channel_id TEXT NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
    message_id TEXT NOT NULL,
    PRIMARY KEY (match_id, channel_id)
) STRICT;

-- Patch notes already seen (posted, or there before the first read).
CREATE TABLE deadlock_patches (
    key          TEXT NOT NULL PRIMARY KEY,
    title        TEXT NOT NULL,
    published_at TEXT NOT NULL,
    seen_at      TEXT NOT NULL
) STRICT;

-- Hero and rank art fetched from deadlock-api's asset hosts, by its URL: kept from the media sweep (cards show it).
CREATE TABLE deadlock_media (
    url    TEXT NOT NULL PRIMARY KEY,
    hash   TEXT NOT NULL,
    width  INTEGER NOT NULL,
    height INTEGER NOT NULL
) STRICT;

-- Small server-wide markers (when rank movers last ran).
CREATE TABLE deadlock_state (
    key   TEXT NOT NULL PRIMARY KEY,
    value TEXT NOT NULL
) STRICT;

-- Each server's Deadlock channel (owner's choice) and which posts go there; no row = no posts.
CREATE TABLE server_deadlock_channels (
    server_id   TEXT NOT NULL PRIMARY KEY REFERENCES servers(id) ON DELETE CASCADE,
    channel_id  TEXT NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
    match_cards INTEGER NOT NULL DEFAULT 1,
    rank_movers INTEGER NOT NULL DEFAULT 1,
    patch_notes INTEGER NOT NULL DEFAULT 1
) STRICT;
