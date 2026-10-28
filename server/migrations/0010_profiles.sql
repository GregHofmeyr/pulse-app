-- Profiles: the card's fields and the name style live on `users`. `avatar_hash` is reused. There
-- are no foreign keys to `media` (SQLite cannot add one to `avatar_hash` without a rebuild), so
-- saving checks that the hashes exist instead. `name_color` is `#rrggbb` or `#rrggbb,#rrggbb`.
ALTER TABLE users ADD COLUMN banner_hash TEXT;
ALTER TABLE users ADD COLUMN theme_top TEXT NOT NULL DEFAULT '#3a3d46';
ALTER TABLE users ADD COLUMN theme_bottom TEXT NOT NULL DEFAULT '#16171b';
ALTER TABLE users ADD COLUMN name_font TEXT;
ALTER TABLE users ADD COLUMN name_color TEXT;
ALTER TABLE users ADD COLUMN pronouns TEXT;
ALTER TABLE users ADD COLUMN bio TEXT;
-- Unused since the media store (0005) took over uploads.
DROP TABLE images;
