-- Which session each LiveKit token was issued to, so revoking a session can remove the
-- participants it admitted. LiveKit identities are per user (`<user>`, `<user>#v`, `<user>#a`),
-- so the last token issued for a room + identity owns it. Server-side only: nothing here goes
-- into a LiveKit token. Rows go with their session.
CREATE TABLE livekit_grants (
    room       TEXT NOT NULL,
    identity   TEXT NOT NULL,
    token_hash TEXT NOT NULL REFERENCES sessions(token_hash) ON DELETE CASCADE,
    issued_at  TEXT NOT NULL,
    PRIMARY KEY (room, identity)
) STRICT;
CREATE INDEX livekit_grants_session ON livekit_grants(token_hash);
