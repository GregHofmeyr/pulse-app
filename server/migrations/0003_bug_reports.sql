-- Bug, crash and unexpected-exit reports from clients (spec 2026-10-12-bug-reports-design).
CREATE TABLE bug_reports (
    id         TEXT PRIMARY KEY,
    user_id    TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at TEXT NOT NULL,
    kind       TEXT NOT NULL CHECK (kind IN ('bug', 'crash', 'unexpected-exit')),
    text       TEXT NOT NULL,
    meta       TEXT NOT NULL,
    log        TEXT NOT NULL,
    seen_at    TEXT
) STRICT;

CREATE INDEX bug_reports_created ON bug_reports (created_at);
