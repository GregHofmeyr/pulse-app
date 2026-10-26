-- Uploaded files, content-addressed by the SHA-256 of the stored bytes. A still first frame is
-- its own row of the same kind; its animated parent points at it through `still_hash`.
CREATE TABLE media (hash TEXT PRIMARY KEY, kind TEXT NOT NULL, location TEXT NOT NULL DEFAULT 'disk',
  content_type TEXT NOT NULL, size INTEGER NOT NULL, width INTEGER NOT NULL, height INTEGER NOT NULL,
  frames INTEGER NOT NULL, still_hash TEXT, uploaded_by TEXT REFERENCES users(id), created_at TEXT NOT NULL) STRICT;
-- `media_hash` is NULL once deleted, so a deleted emoji keeps its name but not its media.
CREATE TABLE custom_emoji (id TEXT PRIMARY KEY, server_id TEXT NOT NULL REFERENCES servers(id) ON DELETE CASCADE,
  name TEXT NOT NULL, media_hash TEXT REFERENCES media(hash), animated INTEGER NOT NULL,
  added_by TEXT REFERENCES users(id), created_at TEXT NOT NULL, deleted_at TEXT) STRICT;
CREATE UNIQUE INDEX custom_emoji_live_name ON custom_emoji(server_id, lower(name)) WHERE deleted_at IS NULL;
CREATE TABLE emoji_usage (user_id TEXT NOT NULL REFERENCES users(id), emoji TEXT NOT NULL, score REAL NOT NULL,
  last_used_at TEXT NOT NULL, PRIMARY KEY (user_id, emoji)) STRICT;
ALTER TABLE channels ADD COLUMN reaction_mode TEXT NOT NULL DEFAULT 'normal';
