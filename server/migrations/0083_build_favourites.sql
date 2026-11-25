-- Build favourites (#617): each person's own starred builds, seen by nobody else. A row goes with its build: the
-- `builds` row is deleted when a build is deleted (the message itself is only soft-deleted) and cascades from its
-- message when that is purged, so referencing it covers both; and with its person. A new table only: nothing is copied,
-- dropped or rebuilt.
-- Rollback = restore the pre-deploy backup (or leave it: an older server never reads it).
CREATE TABLE build_favourites (
    user_id    TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    message_id TEXT NOT NULL REFERENCES builds(message_id) ON DELETE CASCADE,
    created_at TEXT NOT NULL,
    PRIMARY KEY (user_id, message_id)
) STRICT, WITHOUT ROWID;
CREATE INDEX build_favourites_message ON build_favourites(message_id);
