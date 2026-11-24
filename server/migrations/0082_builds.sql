-- Builds (#616): a `builds` channel kind and the `builds` rows. A build is a message of a builds channel plus one row
-- here, as a clip is; its screenshots are the message's attachments and its comments are thread replies.
--
-- A CHECK can't be altered in SQLite, so `channels` is rebuilt again, as 0008 and 0009 did, now with every column it
-- has gained since. `db::connect` runs migrations with foreign keys OFF; the guard below makes the migration fail,
-- rolled back, if it is ever run with them on.
CREATE TEMP TABLE migration_guard (ok INTEGER NOT NULL CHECK (ok = 0));
INSERT INTO migration_guard SELECT foreign_keys FROM pragma_foreign_keys;

CREATE TABLE channels_new (
    id            TEXT PRIMARY KEY,
    server_id     TEXT REFERENCES servers(id) ON DELETE CASCADE,
    kind          TEXT NOT NULL
                  CHECK (kind IN ('text', 'voice', 'dm', 'group', 'quotes', 'clips', 'builds')),
    name          TEXT,
    position      INTEGER NOT NULL DEFAULT 0,
    created_at    TEXT NOT NULL,
    reaction_mode TEXT NOT NULL DEFAULT 'normal',
    deleted_at    TEXT,
    deleted_by    TEXT REFERENCES users(id),
    icon_hash     TEXT,
    category_id   TEXT REFERENCES categories(id) ON DELETE SET NULL,
    bitrate       INTEGER,
    archived_at   TEXT,
    CHECK ((kind IN ('dm', 'group')) = (server_id IS NULL))
) STRICT;
INSERT INTO channels_new (id, server_id, kind, name, position, created_at, reaction_mode, deleted_at, deleted_by,
                          icon_hash, category_id, bitrate, archived_at)
    SELECT id, server_id, kind, name, position, created_at, reaction_mode, deleted_at, deleted_by,
           icon_hash, category_id, bitrate, archived_at
    FROM channels;
DROP TABLE channels;
ALTER TABLE channels_new RENAME TO channels;
CREATE INDEX channels_server ON channels(server_id);
CREATE INDEX channels_deleted ON channels(deleted_at) WHERE deleted_at IS NOT NULL;
CREATE INDEX channels_category ON channels(category_id);

-- Nothing may reference a channel row that is gone, and the new table must be sound: any violation in `channels`,
-- `categories` or the tables that reference `channels` fails the migration. Scoped on purpose: an unrelated orphan row
-- elsewhere must not stop the server booting.
INSERT INTO migration_guard
    SELECT 1 FROM (
        SELECT 1 FROM pragma_foreign_key_check('channels')
        UNION ALL SELECT 1 FROM pragma_foreign_key_check('categories')
        UNION ALL SELECT 1 FROM pragma_foreign_key_check('channel_members')
        UNION ALL SELECT 1 FROM pragma_foreign_key_check('messages')
        UNION ALL SELECT 1 FROM pragma_foreign_key_check('read_states')
        UNION ALL SELECT 1 FROM pragma_foreign_key_check('voice_sessions')
        UNION ALL SELECT 1 FROM pragma_foreign_key_check('dm_hidden')
        UNION ALL SELECT 1 FROM pragma_foreign_key_check('clip_uploads')
        UNION ALL SELECT 1 FROM pragma_foreign_key_check('clip_view_prefs')
        UNION ALL SELECT 1 FROM pragma_foreign_key_check('stream_sessions')
        UNION ALL SELECT 1 FROM pragma_foreign_key_check('calls')
        UNION ALL SELECT 1 FROM pragma_foreign_key_check('pins')
        UNION ALL SELECT 1 FROM pragma_foreign_key_check('wheel_spins')
        UNION ALL SELECT 1 FROM pragma_foreign_key_check('polls')
        UNION ALL SELECT 1 FROM pragma_foreign_key_check('attachments')
        UNION ALL SELECT 1 FROM pragma_foreign_key_check('server_achievement_channels')
        UNION ALL SELECT 1 FROM pragma_foreign_key_check('plans')
        UNION ALL SELECT 1 FROM pragma_foreign_key_check('deadlock_match_posts')
        UNION ALL SELECT 1 FROM pragma_foreign_key_check('server_deadlock_channels')
        UNION ALL SELECT 1 FROM pragma_foreign_key_check('server_f1_channels')
        UNION ALL SELECT 1 FROM pragma_foreign_key_check('server_sports_channels')
        UNION ALL SELECT 1 FROM pragma_foreign_key_check('sports_posts')
        UNION ALL SELECT 1 FROM pragma_foreign_key_check('scheduled_messages')
        UNION ALL SELECT 1 FROM pragma_foreign_key_check('dm_placements')
        UNION ALL SELECT 1 FROM pragma_foreign_key_check('watch_party_snapshots')
        UNION ALL SELECT 1 FROM pragma_foreign_key_check('game_invites')
    ) LIMIT 1;
DROP TABLE migration_guard;

-- One row per build. `channel_id` repeats the message's so the browse and by-game lists have their indexes; search
-- needs none (the message's `content` is the build's plain-text rendering, indexed by the message triggers).
-- `title_auto`: the title was made for the author (from the link, or "Untitled build") and the link's fetched title
-- may replace it. `link_*` are what the server fetched of the link's page. `quick_drop`: saved with no details yet.
CREATE TABLE builds (
    message_id       TEXT PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
    channel_id       TEXT NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
    title            TEXT NOT NULL,
    title_auto       INTEGER NOT NULL DEFAULT 0,
    game_id          TEXT REFERENCES games(id),
    game_guessed     INTEGER NOT NULL DEFAULT 0,
    link_url         TEXT,
    link_title       TEXT,
    link_thumb       TEXT REFERENCES media(hash),
    link_provider    TEXT,
    link_provider_id TEXT,
    notes            TEXT,
    quick_drop       INTEGER NOT NULL,
    updated_at       TEXT,
    archived_at      TEXT,
    archived_by      TEXT REFERENCES users(id),
    CHECK ((archived_at IS NULL) = (archived_by IS NULL))
) STRICT;
CREATE INDEX builds_browse ON builds(channel_id, archived_at, message_id);
CREATE INDEX builds_game ON builds(channel_id, game_id, message_id);
