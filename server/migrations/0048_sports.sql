-- Sports score pings (#385): match-day banners, a live score edited in place and full-time cards for the teams people
-- follow, from API-Sports (api-football and api-rugby). Additive: new tables only (no rebuilds, so foreign keys being
-- OFF while migrating changes nothing). Nothing is fetched unless the server has an API-Sports key, someone follows a
-- team and a server's owner picks a Sports channel. Rollback = leave them (older servers never read them) or drop the
-- eight tables.

-- Each person's "Sports score pings" switch (no row = on): off, their follows bring no cards.
CREATE TABLE sports_prefs (
    user_id TEXT NOT NULL PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    pings   INTEGER NOT NULL DEFAULT 1
) STRICT;

-- The teams each person follows; `league` is psl, urc or epl, `team_id` the provider's.
CREATE TABLE sports_follows (
    user_id    TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    league     TEXT NOT NULL,
    team_id    INTEGER NOT NULL,
    team_name  TEXT NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY (user_id, league, team_id)
) STRICT;
CREATE INDEX sports_follows_team ON sports_follows (league, team_id);

-- Each league's teams this season, as last fetched (a week at a time): the follow search reads these.
CREATE TABLE sports_teams (
    league  TEXT NOT NULL,
    team_id INTEGER NOT NULL,
    name    TEXT NOT NULL,
    PRIMARY KEY (league, team_id)
) STRICT;

-- Upcoming and recent matches of the followed leagues, as last read. `kickoff` is unix seconds; `scorers` a JSON
-- array; `done` = full time, called off, or past its window (no more reads).
CREATE TABLE sports_fixtures (
    league     TEXT NOT NULL,
    fixture_id INTEGER NOT NULL,
    kickoff    INTEGER NOT NULL,
    home_id    INTEGER NOT NULL,
    home_name  TEXT NOT NULL,
    away_id    INTEGER NOT NULL,
    away_name  TEXT NOT NULL,
    status     TEXT NOT NULL,
    home_score INTEGER,
    away_score INTEGER,
    minute     INTEGER,
    scorers    TEXT NOT NULL DEFAULT '[]',
    done       INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (league, fixture_id)
) STRICT;
CREATE INDEX sports_fixtures_kickoff ON sports_fixtures (done, kickoff);

-- The card posted for a match in a channel: one per match, edited in place.
CREATE TABLE sports_posts (
    league     TEXT NOT NULL,
    fixture_id INTEGER NOT NULL,
    channel_id TEXT NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
    message_id TEXT NOT NULL,
    PRIMARY KEY (league, fixture_id, channel_id)
) STRICT;

-- Requests made to API-Sports per sport (football, rugby) per UTC day: the free plan allows 100 a day each and the
-- server stops well short of that.
CREATE TABLE sports_quota (
    sport TEXT NOT NULL,
    day   TEXT NOT NULL,
    used  INTEGER NOT NULL,
    PRIMARY KEY (sport, day)
) STRICT;

-- Small markers (when each league's schedule and teams were last read).
CREATE TABLE sports_state (
    key   TEXT NOT NULL PRIMARY KEY,
    value TEXT NOT NULL
) STRICT;

-- Each server's Sports channel (owner's choice); no row = no cards.
CREATE TABLE server_sports_channels (
    server_id  TEXT NOT NULL PRIMARY KEY REFERENCES servers(id) ON DELETE CASCADE,
    channel_id TEXT NOT NULL REFERENCES channels(id) ON DELETE CASCADE
) STRICT;
