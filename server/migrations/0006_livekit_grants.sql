-- LiveKit grants: every voice token / stream ticket gets a random id, sent to LiveKit as the
-- participant attribute `pulse_grant` (random, so it reveals nothing; never the session hash).
-- A join is accepted only for the grant of a live session issued for that room + identity, and
-- revoking a session removes the participants LiveKit reports carrying one of its grants.
-- `joined_sid` / `left_at` track the participant connected with a grant, so unused and finished
-- grants can be pruned once their token is useless. Rows go with their session.
CREATE TABLE livekit_grants (
    id         TEXT PRIMARY KEY,
    token_hash TEXT NOT NULL REFERENCES sessions(token_hash) ON DELETE CASCADE,
    room       TEXT NOT NULL,
    identity   TEXT NOT NULL,
    issued_at  TEXT NOT NULL,
    joined_sid TEXT,
    left_at    TEXT
) STRICT;
CREATE INDEX livekit_grants_session ON livekit_grants(token_hash);
CREATE INDEX livekit_grants_issued ON livekit_grants(issued_at);
