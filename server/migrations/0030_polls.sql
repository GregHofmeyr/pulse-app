-- Polls (#157). New tables only (no rebuilds), so foreign keys being OFF while migrating changes nothing.
-- Rollback = restore the pre-deploy backup (or drop both tables: a poll's message keeps its plain-text fallback, and
-- its "ended" line carries its own copy of the result).

-- A poll, keyed by its message (an ordinary message whose content is the poll as text, for clients that don't know
-- polls). `options` is a JSON array of strings. `closed_at` is set once, by its creator ending it or by the sweep
-- after `ends_at`; whoever sets it posts the "ended" line. Goes with its message or its chat.
CREATE TABLE polls (
    message_id TEXT PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
    channel_id TEXT NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
    question   TEXT NOT NULL,
    options    TEXT NOT NULL,
    multi      INTEGER NOT NULL,
    anonymous  INTEGER NOT NULL,
    ends_at    TEXT,
    closed_at  TEXT,
    created_at TEXT NOT NULL
) STRICT;

-- The sweep's look for open polls whose time is up.
CREATE INDEX polls_due ON polls(ends_at) WHERE closed_at IS NULL AND ends_at IS NOT NULL;

-- One row per person per option they voted for. Single choice (at most one row per person) is kept by the vote
-- route, which replaces a person's rows in one write transaction.
CREATE TABLE poll_votes (
    message_id TEXT NOT NULL REFERENCES polls(message_id) ON DELETE CASCADE,
    option     INTEGER NOT NULL,
    voter_id   TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    voted_at   TEXT NOT NULL,
    PRIMARY KEY (message_id, voter_id, option)
) STRICT;

-- Who has voted at some point, so taking your vote back doesn't hide results you've already seen (and can't be used
-- to peek at them without voting).
CREATE TABLE poll_seen (
    message_id TEXT NOT NULL REFERENCES polls(message_id) ON DELETE CASCADE,
    user_id    TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    PRIMARY KEY (message_id, user_id)
) STRICT;
