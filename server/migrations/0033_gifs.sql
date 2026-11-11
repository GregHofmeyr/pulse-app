-- GIFs (#58). Two additions, nothing rebuilt or copied:
--
-- 1. messages.gif: a GIF sent from the picker, as JSON `{"id","title","width","height"}` (GIPHY's id and what to show
--    before it loads; never GIPHY URLs, which GIPHY's terms forbid storing). NULL for every other message, which every
--    existing row gets. The content holds the GIF's GIPHY link, so older clients show a link.
-- 2. gif_favourites: each person's starred GIFs, newest first, at most 200 (checked by the route). A row goes with its
--    person (ON DELETE CASCADE).
--
-- ADD COLUMN and CREATE TABLE are not table rebuilds, so foreign keys being OFF while migrating (db.rs) changes
-- nothing: no row is copied, dropped or cascaded.
-- Rollback = restore the pre-deploy backup (or ignore both: older servers never read them).
ALTER TABLE messages ADD COLUMN gif TEXT;

CREATE TABLE gif_favourites (
    user_id    TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    gif_id     TEXT NOT NULL,
    title      TEXT NOT NULL,
    width      INTEGER NOT NULL,
    height     INTEGER NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY (user_id, gif_id)
) STRICT;
CREATE INDEX gif_favourites_newest ON gif_favourites (user_id, created_at);
