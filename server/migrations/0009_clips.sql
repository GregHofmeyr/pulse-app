-- Clips: a `clips` channel kind, the games registry, clip rows, upload tickets and per-user
-- view preferences.
--
-- A CHECK can't be altered in SQLite, so `channels` is rebuilt again, exactly as 0008 did (see
-- there). `db::connect` runs migrations with foreign keys OFF; the guard below makes the
-- migration fail, rolled back, if it is ever run with them on.
CREATE TEMP TABLE migration_guard (ok INTEGER NOT NULL CHECK (ok = 0));
INSERT INTO migration_guard SELECT foreign_keys FROM pragma_foreign_keys;

CREATE TABLE channels_new (
    id            TEXT PRIMARY KEY,
    server_id     TEXT REFERENCES servers(id) ON DELETE CASCADE,
    kind          TEXT NOT NULL CHECK (kind IN ('text', 'voice', 'dm', 'group', 'quotes', 'clips')),
    name          TEXT,
    position      INTEGER NOT NULL DEFAULT 0,
    created_at    TEXT NOT NULL,
    reaction_mode TEXT NOT NULL DEFAULT 'normal',
    CHECK ((kind IN ('dm', 'group')) = (server_id IS NULL))
) STRICT;
INSERT INTO channels_new (id, server_id, kind, name, position, created_at, reaction_mode)
    SELECT id, server_id, kind, name, position, created_at, reaction_mode FROM channels;
DROP TABLE channels;
ALTER TABLE channels_new RENAME TO channels;
CREATE INDEX channels_server ON channels(server_id);

-- Nothing may reference a channel row that is gone, and the new table must be sound: any
-- violation in `channels` or the tables that reference it fails the migration. Scoped on purpose:
-- an unrelated orphan row elsewhere must not stop the server booting.
INSERT INTO migration_guard
    SELECT 1 FROM (
        SELECT 1 FROM pragma_foreign_key_check('channels')
        UNION ALL SELECT 1 FROM pragma_foreign_key_check('channel_members')
        UNION ALL SELECT 1 FROM pragma_foreign_key_check('messages')
        UNION ALL SELECT 1 FROM pragma_foreign_key_check('read_states')
        UNION ALL SELECT 1 FROM pragma_foreign_key_check('dm_hidden')
        UNION ALL SELECT 1 FROM pragma_foreign_key_check('voice_sessions')
    ) LIMIT 1;
DROP TABLE migration_guard;

-- Per-server game registry. `name_key` is the normalised name (Unicode lowercase, trimmed,
-- whitespace collapsed), unique per server.
CREATE TABLE games (
    id         TEXT PRIMARY KEY,
    server_id  TEXT NOT NULL REFERENCES servers(id) ON DELETE CASCADE,
    name       TEXT NOT NULL,
    name_key   TEXT NOT NULL,
    created_by TEXT REFERENCES users(id),
    created_at TEXT NOT NULL
) STRICT;
CREATE UNIQUE INDEX games_name ON games(server_id, name_key);

-- A clip is a message of a clips channel with one row here. `video_key` is the `media` row of
-- the video (a server-assigned random key, stored in R2); `thumb_hash` is its thumbnail.
CREATE TABLE clips (
    message_id  TEXT PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
    video_key   TEXT NOT NULL UNIQUE REFERENCES media(hash),
    thumb_hash  TEXT NOT NULL REFERENCES media(hash),
    duration_ms INTEGER NOT NULL,
    width       INTEGER NOT NULL,
    height      INTEGER NOT NULL,
    game_id     TEXT REFERENCES games(id)
) STRICT;
CREATE INDEX clips_game ON clips(game_id);

-- An invisible upload ticket: records the object key, size and owner until the upload is
-- confirmed or swept. SET NULL, not CASCADE: a ticket must outlive its channel, or the sweeper
-- would lose the key of an object that was already uploaded.
CREATE TABLE clip_uploads (
    id          TEXT PRIMARY KEY,
    key         TEXT NOT NULL UNIQUE,
    user_id     TEXT NOT NULL REFERENCES users(id),
    channel_id  TEXT REFERENCES channels(id) ON DELETE SET NULL,
    size        INTEGER NOT NULL,
    duration_ms INTEGER NOT NULL,
    created_at  TEXT NOT NULL
) STRICT;
CREATE INDEX clip_uploads_user ON clip_uploads(user_id, created_at);

CREATE TABLE clip_view_prefs (
    user_id    TEXT NOT NULL REFERENCES users(id),
    channel_id TEXT NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
    view       TEXT NOT NULL CHECK (view IN ('list', 'wall')),
    PRIMARY KEY (user_id, channel_id)
) STRICT;

-- The hourly clip count looks up one author's recent top-level messages.
CREATE INDEX messages_author_recent ON messages(author_id, created_at) WHERE thread_root_id IS NULL;
