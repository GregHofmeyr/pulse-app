-- Sessions an owner tool revoked from its own process (#466): `reset-password` and `revoke-sessions` delete a person's
-- sessions, but the running server never hears of it. They note each revoked session here (and the LiveKit grants it
-- held, which go with the session by cascade) in the same transaction; the running server's revoked-session sweeper
-- takes the notes, closes those sessions' sockets (4001: the app goes to sign-in) and removes their call and stream
-- connections. A session that merely expires is never noted, so nobody is kicked by expiry.
-- Rollback = nothing to undo: an older server ignores these tables (its sweeper still ends ghost watches).
CREATE TABLE revoked_sessions (
    token_hash TEXT PRIMARY KEY,
    revoked_at TEXT NOT NULL
) STRICT;

CREATE TABLE revoked_grants (
    id         TEXT PRIMARY KEY,
    token_hash TEXT NOT NULL REFERENCES revoked_sessions(token_hash) ON DELETE CASCADE,
    room       TEXT NOT NULL
) STRICT;
CREATE INDEX revoked_grants_session ON revoked_grants(token_hash);
