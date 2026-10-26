-- Per-token LiveKit grants (replaces 0006's last-issued-wins table, which never shipped).
-- Every voice token / stream ticket gets a random id, sent to LiveKit as the participant
-- attribute `pulse_grant` (random, so it reveals nothing; never the session hash). A join is
-- accepted only for the grant of a live session matching its room + identity, and
-- `livekit_joined` remembers which grant the participant in each room + identity joined with
-- (the latest join wins, as in LiveKit, which drops the older duplicate). Revoking a session
-- removes exactly the participants that joined with its grants. Rows go with their session.
DROP TABLE livekit_grants;

CREATE TABLE livekit_grants (
    id         TEXT PRIMARY KEY,
    token_hash TEXT NOT NULL REFERENCES sessions(token_hash) ON DELETE CASCADE,
    room       TEXT NOT NULL,
    identity   TEXT NOT NULL,
    issued_at  TEXT NOT NULL
) STRICT;
CREATE INDEX livekit_grants_session ON livekit_grants(token_hash, issued_at);

CREATE TABLE livekit_joined (
    room      TEXT NOT NULL,
    identity  TEXT NOT NULL,
    grant_id  TEXT NOT NULL REFERENCES livekit_grants(id) ON DELETE CASCADE,
    joined_at TEXT NOT NULL,
    PRIMARY KEY (room, identity)
) STRICT;
CREATE INDEX livekit_joined_grant ON livekit_joined(grant_id);
