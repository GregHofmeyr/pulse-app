-- Quotes: a `quotes` channel kind, quote posts with lines and awards, threads, speaker links and
-- Quote of the Year.
--
-- A CHECK can't be altered in SQLite, so `channels` is rebuilt (the documented 12-step way: new
-- table, copy, drop, rename). Dropping a parent table with foreign keys ON runs its ON DELETE
-- CASCADEs and would wipe every message, so `db::connect` runs migrations on a connection with
-- foreign keys OFF (sqlx's SQLite driver always wraps a migration in a transaction, where
-- `PRAGMA foreign_keys` can't be changed, so the migration can't switch them itself). The guard
-- below makes the migration fail, rolled back, if it is ever run with them on.
CREATE TEMP TABLE migration_guard (ok INTEGER NOT NULL CHECK (ok = 0));
INSERT INTO migration_guard SELECT foreign_keys FROM pragma_foreign_keys;

CREATE TABLE channels_new (
    id            TEXT PRIMARY KEY,
    server_id     TEXT REFERENCES servers(id) ON DELETE CASCADE,
    kind          TEXT NOT NULL CHECK (kind IN ('text', 'voice', 'dm', 'group', 'quotes')),
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

-- Nothing may reference a channel row that is gone: any violation fails the migration.
INSERT INTO migration_guard SELECT 1 FROM pragma_foreign_key_check LIMIT 1;
DROP TABLE migration_guard;

-- A quote is a message of a quotes channel with one row here. `posted_on` is the day it was
-- posted (client-local, clamped to within a day of UTC); `said_on` is import-only. Year =
-- COALESCE(said_on, posted_on).
CREATE TABLE quotes (
    message_id TEXT PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
    posted_on  TEXT NOT NULL,
    said_on    TEXT,
    year       INTEGER NOT NULL
) STRICT;
CREATE INDEX quotes_year ON quotes(year);

-- A line is a speaker (a member, or a plain name matched through `speaker_key`) saying text, or
-- an image with an optional caption.
CREATE TABLE quote_lines (
    id           TEXT PRIMARY KEY,
    message_id   TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    position     INTEGER NOT NULL,
    speaker_id   TEXT REFERENCES users(id),
    speaker_name TEXT,
    speaker_key  TEXT,
    text         TEXT,
    media_hash   TEXT REFERENCES media(hash),
    caption      TEXT,
    CHECK ((speaker_id IS NULL) != (speaker_name IS NULL)),
    CHECK ((text IS NULL) != (media_hash IS NULL)),
    CHECK (caption IS NULL OR media_hash IS NOT NULL),
    CHECK ((speaker_key IS NOT NULL) = (speaker_name IS NOT NULL))
) STRICT;
CREATE INDEX quote_lines_message ON quote_lines(message_id, position);
CREATE INDEX quote_lines_speaker_key ON quote_lines(speaker_key);

CREATE TABLE quote_awards (
    line_id    TEXT NOT NULL REFERENCES quote_lines(id) ON DELETE CASCADE,
    giver_id   TEXT NOT NULL REFERENCES users(id),
    created_at TEXT NOT NULL,
    PRIMARY KEY (line_id, giver_id)
) STRICT;

-- Replies to a quote are threads.
ALTER TABLE messages ADD COLUMN thread_root_id TEXT REFERENCES messages(id);
CREATE INDEX messages_thread ON messages(thread_root_id, id) WHERE thread_root_id IS NOT NULL;

-- Per user: how far into a quote's thread they have read.
CREATE TABLE thread_reads (
    user_id      TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    root_id      TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    last_seen_id TEXT NOT NULL,
    PRIMARY KEY (user_id, root_id)
) STRICT;

-- "Plain name X means member Y", per server; lines keep their raw speaker.
CREATE TABLE speaker_links (
    server_id TEXT NOT NULL REFERENCES servers(id) ON DELETE CASCADE,
    name_key  TEXT NOT NULL,
    name      TEXT NOT NULL,
    user_id   TEXT NOT NULL REFERENCES users(id),
    linked_by TEXT NOT NULL REFERENCES users(id),
    linked_at TEXT NOT NULL,
    PRIMARY KEY (server_id, name_key)
) STRICT;

-- Quote of the Year: one event per (server, year), at most one open per server.
CREATE TABLE qoty_events (
    id         TEXT PRIMARY KEY,
    server_id  TEXT NOT NULL REFERENCES servers(id) ON DELETE CASCADE,
    year       INTEGER NOT NULL,
    started_by TEXT NOT NULL REFERENCES users(id),
    started_at TEXT NOT NULL,
    ends_at    TEXT,
    ended_at   TEXT,
    ended_by   TEXT REFERENCES users(id),
    end_reason TEXT CHECK (end_reason IN ('early', 'time')),
    UNIQUE (server_id, year)
) STRICT;
CREATE UNIQUE INDEX qoty_events_open ON qoty_events(server_id) WHERE ended_at IS NULL;

CREATE TABLE qoty_options (
    event_id   TEXT NOT NULL REFERENCES qoty_events(id) ON DELETE CASCADE,
    message_id TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    added_by   TEXT REFERENCES users(id),
    PRIMARY KEY (event_id, message_id)
) STRICT;

-- Votes are public; one per person per event. Removing an option removes its votes.
CREATE TABLE qoty_votes (
    event_id   TEXT NOT NULL,
    user_id    TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    message_id TEXT NOT NULL,
    PRIMARY KEY (event_id, user_id),
    FOREIGN KEY (event_id, message_id) REFERENCES qoty_options(event_id, message_id) ON DELETE CASCADE
) STRICT;

-- What won, kept when the quote is later deleted (the speakers are snapshotted beside it).
CREATE TABLE qoty_winners (
    event_id   TEXT NOT NULL REFERENCES qoty_events(id) ON DELETE CASCADE,
    message_id TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    votes      INTEGER NOT NULL,
    PRIMARY KEY (event_id, message_id)
) STRICT;

CREATE TABLE qoty_winner_speakers (
    event_id   TEXT NOT NULL,
    message_id TEXT NOT NULL,
    position   INTEGER NOT NULL,
    user_id    TEXT REFERENCES users(id),
    name       TEXT,
    PRIMARY KEY (event_id, message_id, position),
    FOREIGN KEY (event_id, message_id) REFERENCES qoty_winners(event_id, message_id) ON DELETE CASCADE
) STRICT;
