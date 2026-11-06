-- Badge art (#225): a picture for a badge, set only with the admin command (`badge art`) and stored in the media store
-- (kind `badge_art`, always a still), so it never lives in the repo. A badge without a row keeps its emoji. The media
-- sweep keeps a hash in use here (media::cleanup).
CREATE TABLE badge_art (
    badge_key  TEXT PRIMARY KEY,
    media_hash TEXT NOT NULL REFERENCES media(hash),
    updated_at TEXT NOT NULL
) STRICT;
