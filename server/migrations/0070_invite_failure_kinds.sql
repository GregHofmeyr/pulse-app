-- Invite pages and sign-up through an invite (#80, step 3): the failure log gets one more kind of entry, for lookups
-- that can only come from a script. SQLite can't change a CHECK, so the table is rebuilt with every row kept. Nothing
-- references it, so foreign keys being OFF while migrating (db.rs) changes nothing.
-- Rollback = restore the pre-deploy backup (an older server reads the table as it was, but refuses to log the new kind).
CREATE TABLE invite_failures_new (
    id         INTEGER PRIMARY KEY,
    at         TEXT NOT NULL,
    ip         TEXT NOT NULL,
    user_id    TEXT REFERENCES users(id) ON DELETE SET NULL,
    user_agent TEXT,
    code       TEXT NOT NULL,
    kind       TEXT NOT NULL CHECK (kind IN ('unknown', 'expired', 'revoked', 'used_up', 'trap'))
) STRICT;
INSERT INTO invite_failures_new (id, at, ip, user_id, user_agent, code, kind)
    SELECT id, at, ip, user_id, user_agent, code, kind FROM invite_failures;
DROP TABLE invite_failures;
ALTER TABLE invite_failures_new RENAME TO invite_failures;
CREATE INDEX invite_failures_ip ON invite_failures (ip, at);
CREATE INDEX invite_failures_user ON invite_failures (user_id, at) WHERE user_id IS NOT NULL;
CREATE INDEX invite_failures_at ON invite_failures (at);
