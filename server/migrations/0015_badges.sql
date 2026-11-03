-- Badges (#49). The server awards every badge from counters bumped as things happen, never from
-- recounting messages or sessions, so progress survives deleted and purged channels. That's why
-- neither `badge_stats` nor `user_badges` references a channel. Server activity only: DM and
-- group activity is never counted.
CREATE TABLE badge_stats (
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    key     TEXT NOT NULL,
    value   INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (user_id, key)
) STRICT;

-- The tier held (1 for a badge without tiers), the place on the profile's showcase (0-4, NULL =
-- not chosen) and whether its owner has opened their trophy case since earning it.
ALTER TABLE user_badges ADD COLUMN tier INTEGER NOT NULL DEFAULT 1;
ALTER TABLE user_badges ADD COLUMN showcase INTEGER;
ALTER TABLE user_badges ADD COLUMN seen INTEGER NOT NULL DEFAULT 0;

-- The first person to reach each tier of each badge, kept forever (the primary key makes a race
-- have exactly one winner).
CREATE TABLE badge_firsts (
    badge_key TEXT NOT NULL,
    tier      INTEGER NOT NULL,
    user_id   TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    earned_at TEXT NOT NULL,
    PRIMARY KEY (badge_key, tier)
) STRICT;

-- Server streams, for "Chat, did you see that?" (hours streamed). Like `voice_sessions`, an open
-- row has no `ended_at`.
CREATE TABLE stream_sessions (
    id         TEXT PRIMARY KEY,
    user_id    TEXT NOT NULL REFERENCES users(id),
    channel_id TEXT NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
    started_at TEXT NOT NULL,
    ended_at   TEXT
) STRICT;
CREATE INDEX stream_sessions_open ON stream_sessions(user_id, channel_id, ended_at);
