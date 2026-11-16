-- F1 race weekends (#386): a countdown, the weekend card with podium predictions, a "lights out in 1 hour" line, and
-- the results and scores after the race, in each server's F1 channel. New tables only (no rebuilds), so foreign keys
-- being OFF while migrating changes nothing. Nobody has an F1 channel yet, so nothing is fetched or posted until a
-- server's owner picks one. Rollback = leave them (older servers never read them; the cards keep their plain-text
-- fallback) or drop the four tables.

-- Each server's F1 channel (owner's choice); no row = no F1 cards.
CREATE TABLE server_f1_channels (
    server_id  TEXT NOT NULL PRIMARY KEY REFERENCES servers(id) ON DELETE CASCADE,
    channel_id TEXT NOT NULL REFERENCES channels(id) ON DELETE CASCADE
) STRICT;

-- Jolpica-F1's answers as the server read them (the season's calendar, a round's results, the standings after a
-- round, the entry list), by what was asked (`schedule:2026`, `results:2026:21`, ...). An empty answer is kept too,
-- so it isn't asked again until it's due. Results and standings are never asked again once they have rows.
CREATE TABLE f1_cache (
    key        TEXT NOT NULL PRIMARY KEY,
    body       TEXT NOT NULL,
    fetched_at TEXT NOT NULL,
    -- when an answer first had rows (results: the standings' grace starts here)
    filled_at  TEXT
) STRICT;

-- What each server was sent for each round: 'weekend' (the card), 'lights_out' (the line), 'results' (the results
-- card, then the scores). A row is written in the same transaction as its line, so a restart neither loses nor
-- repeats one. `message_id` is the line (NULL when it was skipped, or the message is gone).
CREATE TABLE f1_posts (
    server_id  TEXT NOT NULL REFERENCES servers(id) ON DELETE CASCADE,
    season     INTEGER NOT NULL,
    round      INTEGER NOT NULL,
    kind       TEXT NOT NULL CHECK (kind IN ('weekend', 'lights_out', 'results')),
    message_id TEXT REFERENCES messages(id) ON DELETE SET NULL,
    posted_at  TEXT NOT NULL,
    PRIMARY KEY (server_id, season, round, kind)
) STRICT;
CREATE INDEX f1_posts_message ON f1_posts (message_id);

-- Podium predictions on a weekend card: one per person, P1-P3 as Jolpica driver ids (a JSON array). `points` is set
-- when the race is scored (3 an exact place, 1 on the podium elsewhere); a season's table is their sum over the
-- server's weekend cards of that season. Goes with the card or the person.
CREATE TABLE f1_predictions (
    message_id TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    user_id    TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    picks      TEXT NOT NULL,
    at         TEXT NOT NULL,
    points     INTEGER,
    PRIMARY KEY (message_id, user_id)
) STRICT;
