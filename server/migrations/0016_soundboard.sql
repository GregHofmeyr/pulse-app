CREATE TABLE sounds (id TEXT PRIMARY KEY, server_id TEXT NOT NULL REFERENCES servers(id) ON DELETE CASCADE,
  name TEXT NOT NULL, emoji TEXT, media_hash TEXT NOT NULL REFERENCES media(hash), volume INTEGER NOT NULL DEFAULT 100,
  duration_ms INTEGER NOT NULL, added_by TEXT REFERENCES users(id), created_at TEXT NOT NULL,
  plays INTEGER NOT NULL DEFAULT 0) STRICT;
CREATE INDEX sounds_server ON sounds(server_id);
CREATE TABLE sound_favourites (user_id TEXT NOT NULL REFERENCES users(id), sound_id TEXT NOT NULL REFERENCES sounds(id) ON DELETE CASCADE,
  created_at TEXT NOT NULL, PRIMARY KEY (user_id, sound_id)) STRICT;
CREATE TABLE sound_usage (user_id TEXT NOT NULL REFERENCES users(id), sound_id TEXT NOT NULL REFERENCES sounds(id) ON DELETE CASCADE,
  score REAL NOT NULL, last_used_at TEXT NOT NULL, PRIMARY KEY (user_id, sound_id)) STRICT;
