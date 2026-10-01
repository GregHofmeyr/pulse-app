-- Pulse initial schema (spec §4). All timestamps are RFC 3339 UTC text. IDs are ULIDs.

CREATE TABLE users (
    id            TEXT PRIMARY KEY,
    username      TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    avatar_hash   TEXT,
    status        TEXT NOT NULL DEFAULT 'online' CHECK (status IN ('online', 'dnd')),
    created_at    TEXT NOT NULL
) STRICT;

CREATE TABLE sessions (
    token_hash   TEXT PRIMARY KEY,
    user_id      TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at   TEXT NOT NULL,
    last_used_at TEXT NOT NULL,
    expires_at   TEXT NOT NULL
) STRICT;
CREATE INDEX sessions_user ON sessions(user_id);

CREATE TABLE invites (
    code       TEXT PRIMARY KEY,
    created_by TEXT REFERENCES users(id),
    created_at TEXT NOT NULL,
    used_by    TEXT REFERENCES users(id),
    used_at    TEXT
) STRICT;

CREATE TABLE servers (
    id         TEXT PRIMARY KEY,
    name       TEXT NOT NULL,
    icon_hash  TEXT,
    created_by TEXT NOT NULL REFERENCES users(id),
    created_at TEXT NOT NULL
) STRICT;

CREATE TABLE server_members (
    server_id TEXT NOT NULL REFERENCES servers(id) ON DELETE CASCADE,
    user_id   TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    nickname  TEXT,
    joined_at TEXT NOT NULL,
    PRIMARY KEY (server_id, user_id)
) STRICT;

CREATE TABLE channels (
    id         TEXT PRIMARY KEY,
    server_id  TEXT REFERENCES servers(id) ON DELETE CASCADE,
    kind       TEXT NOT NULL CHECK (kind IN ('text', 'voice', 'dm', 'group')),
    name       TEXT,
    position   INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    CHECK ((kind IN ('dm', 'group')) = (server_id IS NULL))
) STRICT;
CREATE INDEX channels_server ON channels(server_id);

CREATE TABLE channel_members (
    channel_id TEXT NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
    user_id    TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    added_by   TEXT REFERENCES users(id),
    added_at   TEXT NOT NULL,
    PRIMARY KEY (channel_id, user_id)
) STRICT;
CREATE INDEX channel_members_user ON channel_members(user_id);

CREATE TABLE nickname_history (
    id             TEXT PRIMARY KEY,
    server_id      TEXT NOT NULL REFERENCES servers(id) ON DELETE CASCADE,
    target_user_id TEXT NOT NULL REFERENCES users(id),
    changed_by     TEXT NOT NULL REFERENCES users(id),
    old_nick       TEXT,
    new_nick       TEXT,
    changed_at     TEXT NOT NULL
) STRICT;
CREATE INDEX nickname_history_target ON nickname_history(server_id, target_user_id);

CREATE TABLE roles (
    id          TEXT PRIMARY KEY,
    server_id   TEXT NOT NULL REFERENCES servers(id) ON DELETE CASCADE,
    name        TEXT NOT NULL,
    color       TEXT NOT NULL,
    position    INTEGER NOT NULL DEFAULT 0,
    hoist       INTEGER NOT NULL DEFAULT 0,
    permissions INTEGER NOT NULL DEFAULT -1
) STRICT;

CREATE TABLE member_roles (
    server_id TEXT NOT NULL,
    user_id   TEXT NOT NULL,
    role_id   TEXT NOT NULL REFERENCES roles(id) ON DELETE CASCADE,
    PRIMARY KEY (server_id, user_id, role_id),
    FOREIGN KEY (server_id, user_id) REFERENCES server_members(server_id, user_id) ON DELETE CASCADE
) STRICT;

CREATE TABLE messages (
    id             TEXT PRIMARY KEY,
    channel_id     TEXT NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
    author_id      TEXT REFERENCES users(id),
    kind           TEXT NOT NULL DEFAULT 'normal' CHECK (kind IN ('normal', 'system')),
    content        TEXT NOT NULL,
    reply_to_id    TEXT REFERENCES messages(id),
    system_payload TEXT,
    created_at     TEXT NOT NULL,
    edited_at      TEXT,
    deleted_at     TEXT
) STRICT;
CREATE INDEX messages_channel ON messages(channel_id, id);

CREATE TABLE reactions (
    message_id TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    user_id    TEXT NOT NULL REFERENCES users(id),
    emoji      TEXT NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY (message_id, user_id, emoji)
) STRICT;

CREATE TABLE mentions (
    message_id TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    user_id    TEXT NOT NULL REFERENCES users(id),
    PRIMARY KEY (message_id, user_id)
) STRICT;
CREATE INDEX mentions_user ON mentions(user_id);

CREATE TABLE read_states (
    user_id              TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    channel_id           TEXT NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
    last_read_message_id TEXT,
    PRIMARY KEY (user_id, channel_id)
) STRICT;

CREATE TABLE notification_prefs (
    user_id     TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    target_kind TEXT NOT NULL CHECK (target_kind IN ('server', 'channel')),
    target_id   TEXT NOT NULL,
    muted       INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (user_id, target_kind, target_id)
) STRICT;

CREATE TABLE images (
    hash       TEXT PRIMARY KEY,
    bytes      BLOB NOT NULL,
    created_at TEXT NOT NULL
) STRICT;

-- Server voice channels only: DM/group calls are never recorded (privacy, spec §8).
CREATE TABLE voice_sessions (
    id         TEXT PRIMARY KEY,
    user_id    TEXT NOT NULL REFERENCES users(id),
    channel_id TEXT NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
    joined_at  TEXT NOT NULL,
    left_at    TEXT
) STRICT;
CREATE INDEX voice_sessions_open ON voice_sessions(user_id, channel_id, left_at);

CREATE TABLE user_badges (
    user_id      TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    badge_key    TEXT NOT NULL,
    earned_at    TEXT NOT NULL,
    first_earner INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (user_id, badge_key)
) STRICT;
