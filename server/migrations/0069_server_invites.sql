-- Server invites and bans (#80, step 1): invite links and direct invites into a server, their use log, the owner's
-- bans, and a log of failed code lookups. New tables only (no rebuilds), so foreign keys being OFF while migrating
-- (db.rs) changes nothing, and older servers never read them. Nothing changes for anyone until an invite is used:
-- servers stay open in this step.
-- Rollback = restore the pre-deploy backup (or leave them: an older server ignores them).

-- An invite into a server. `code` is 10 random characters from an alphabet without look-alikes. `for_user` set: a
-- direct invite, for that person only (single use), whose card is `message_id` in its maker's DM with them.
-- `expires_at` NULL = never; `max_uses` NULL = no limit (only the owner makes those). `revoked_at` set by Revoke (or a
-- ban); the row is kept so a card can still say why it no longer works, and swept 30 days after it stops working.
-- `signup_ok`: the owner's links may later double as sign-up codes (step 3 reads it).
CREATE TABLE server_invites (
    code       TEXT PRIMARY KEY,
    server_id  TEXT NOT NULL REFERENCES servers(id) ON DELETE CASCADE,
    created_by TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    for_user   TEXT REFERENCES users(id) ON DELETE CASCADE,
    join_text  TEXT,
    created_at TEXT NOT NULL,
    expires_at TEXT,
    max_uses   INTEGER,
    uses       INTEGER NOT NULL DEFAULT 0,
    revoked_at TEXT,
    signup_ok  INTEGER NOT NULL DEFAULT 0 CHECK (signup_ok IN (0, 1)),
    message_id TEXT REFERENCES messages(id) ON DELETE SET NULL
) STRICT;
CREATE INDEX server_invites_server ON server_invites (server_id, created_at);
CREATE INDEX server_invites_maker ON server_invites (created_by, created_at);
CREATE INDEX server_invites_for ON server_invites (for_user) WHERE for_user IS NOT NULL;
CREATE INDEX server_invites_message ON server_invites (message_id) WHERE message_id IS NOT NULL;

-- Who joined through which invite, counted in the same write transaction as the membership. `created_account`: the
-- invite also signed them up (step 3).
CREATE TABLE server_invite_uses (
    code            TEXT NOT NULL REFERENCES server_invites(code) ON DELETE CASCADE,
    user_id         TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    used_at         TEXT NOT NULL,
    created_account INTEGER NOT NULL DEFAULT 0 CHECK (created_account IN (0, 1)),
    PRIMARY KEY (code, user_id)
) STRICT;

-- People a server's owner banned: their invites are refused. Unban deletes the row.
CREATE TABLE server_bans (
    server_id TEXT NOT NULL REFERENCES servers(id) ON DELETE CASCADE,
    user_id   TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    banned_by TEXT REFERENCES users(id) ON DELETE SET NULL,
    banned_at TEXT NOT NULL,
    PRIMARY KEY (server_id, user_id)
) STRICT;

-- Invite codes looked up that didn't work: unknown, expired, revoked or used up. Kept for security only (never
-- shared) and swept after 90 days. `ip` is the address as the rate limits count it.
CREATE TABLE invite_failures (
    id         INTEGER PRIMARY KEY,
    at         TEXT NOT NULL,
    ip         TEXT NOT NULL,
    user_id    TEXT REFERENCES users(id) ON DELETE SET NULL,
    user_agent TEXT,
    code       TEXT NOT NULL,
    kind       TEXT NOT NULL CHECK (kind IN ('unknown', 'expired', 'revoked', 'used_up'))
) STRICT;
CREATE INDEX invite_failures_ip ON invite_failures (ip, at);
CREATE INDEX invite_failures_user ON invite_failures (user_id, at) WHERE user_id IS NOT NULL;
CREATE INDEX invite_failures_at ON invite_failures (at);
