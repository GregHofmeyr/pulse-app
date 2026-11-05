-- Calls in DMs and groups (#50). A call starts when a conversation's voice room goes from empty to occupied and ends
-- once it has been empty for 30 s (`ended_at` is when it went empty). `line_id` is its line in the conversation
-- ("sam started a call"), whose system payload carries the duration and who joined once it ends. Server voice
-- channels never have calls. Rings are in memory only. New tables only: rollback = restore the pre-deploy backup
-- (or drop both tables; nothing else references them).
CREATE TABLE calls (
    id         TEXT PRIMARY KEY,
    channel_id TEXT NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
    started_by TEXT REFERENCES users(id) ON DELETE SET NULL,
    started_at TEXT NOT NULL,
    ended_at   TEXT,
    line_id    TEXT REFERENCES messages(id) ON DELETE SET NULL
) STRICT;

-- At most one open call per conversation.
CREATE UNIQUE INDEX calls_one_open ON calls(channel_id) WHERE ended_at IS NULL;
CREATE INDEX calls_line ON calls(line_id);

-- Everyone who was in the call at some point (the first time they joined): they never see "Missed call".
CREATE TABLE call_participants (
    call_id   TEXT NOT NULL REFERENCES calls(id) ON DELETE CASCADE,
    user_id   TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    joined_at TEXT NOT NULL,
    PRIMARY KEY (call_id, user_id)
) STRICT;
