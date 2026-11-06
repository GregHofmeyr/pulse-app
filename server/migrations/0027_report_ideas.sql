-- Ideas (#218): feature requests from the Report a bug dialog's Idea tab, stored as their own report kind so
-- `reports` lists them apart from bugs.
--
-- A CHECK can't be altered in SQLite, so `bug_reports` is rebuilt, as 0008 and 0009 did for `channels`.
-- Nothing references a report row; the rebuilt table keeps its own link to `users`. `db::connect` runs
-- migrations with foreign keys OFF; the guard below makes the migration fail, rolled back, if it is ever
-- run with them on, as the other rebuilds do.
CREATE TEMP TABLE migration_guard (ok INTEGER NOT NULL CHECK (ok = 0));
INSERT INTO migration_guard SELECT foreign_keys FROM pragma_foreign_keys;

CREATE TABLE bug_reports_new (
    id         TEXT PRIMARY KEY,
    user_id    TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at TEXT NOT NULL,
    kind       TEXT NOT NULL CHECK (kind IN ('bug', 'crash', 'unexpected-exit', 'idea')),
    text       TEXT NOT NULL,
    meta       TEXT NOT NULL,
    log        TEXT NOT NULL,
    seen_at    TEXT
) STRICT;
INSERT INTO bug_reports_new (id, user_id, created_at, kind, text, meta, log, seen_at)
    SELECT id, user_id, created_at, kind, text, meta, log, seen_at FROM bug_reports;
DROP TABLE bug_reports;
ALTER TABLE bug_reports_new RENAME TO bug_reports;
CREATE INDEX bug_reports_created ON bug_reports (created_at);

-- Every copied row must still point at a user: a violation fails the migration.
INSERT INTO migration_guard SELECT 1 FROM pragma_foreign_key_check('bug_reports') LIMIT 1;
DROP TABLE migration_guard;
