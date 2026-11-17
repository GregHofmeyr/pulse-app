-- F1 and sports follow-ups (#403, #406).
--
-- F1:
-- - A sprint's results card goes out once per server per round: `f1_posts.kind` gains 'sprint'. A CHECK can't be
--   changed in SQLite, so `f1_posts` is rebuilt (no table points at it). `db::connect` runs migrations with foreign
--   keys OFF; the guard below makes the migration fail, rolled back, if it is ever run with them on.
-- - Each server's owner picks when podium predictions close: 'lights_out' (the default, as before) or 'qualifying'.
--
-- Sports:
-- - Each server's owner can limit which leagues post in its Sports channel: `leagues` is a JSON array of league keys
--   (`["psl","urc"]`), NULL = every league (the default, as before).
-- - Team logos fetched from API-Sports' media host, by URL, kept from the media sweep (cards show them), as
--   `deadlock_media` keeps Deadlock's art.
--
-- Rollback = restore the pre-deploy backup (an older server reads the rebuilt `f1_posts` as before unless a sprint row
-- was written, and ignores the new columns and table).
CREATE TEMP TABLE migration_guard (ok INTEGER NOT NULL CHECK (ok = 0));
INSERT INTO migration_guard SELECT foreign_keys FROM pragma_foreign_keys;

CREATE TABLE f1_posts_new (
    server_id  TEXT NOT NULL REFERENCES servers(id) ON DELETE CASCADE,
    season     INTEGER NOT NULL,
    round      INTEGER NOT NULL,
    kind       TEXT NOT NULL CHECK (kind IN ('weekend', 'lights_out', 'results', 'sprint')),
    message_id TEXT REFERENCES messages(id) ON DELETE SET NULL,
    posted_at  TEXT NOT NULL,
    PRIMARY KEY (server_id, season, round, kind)
) STRICT;
INSERT INTO f1_posts_new (server_id, season, round, kind, message_id, posted_at)
    SELECT server_id, season, round, kind, message_id, posted_at FROM f1_posts;
DROP TABLE f1_posts;
ALTER TABLE f1_posts_new RENAME TO f1_posts;
CREATE INDEX f1_posts_message ON f1_posts (message_id);

INSERT INTO migration_guard SELECT 1 FROM pragma_foreign_key_check('f1_posts') LIMIT 1;
DROP TABLE migration_guard;

ALTER TABLE server_f1_channels ADD COLUMN picks_close TEXT NOT NULL DEFAULT 'lights_out'
    CHECK (picks_close IN ('lights_out', 'qualifying'));

ALTER TABLE server_sports_channels ADD COLUMN leagues TEXT;

CREATE TABLE sports_media (
    url    TEXT NOT NULL PRIMARY KEY,
    hash   TEXT NOT NULL,
    width  INTEGER NOT NULL,
    height INTEGER NOT NULL
) STRICT;
