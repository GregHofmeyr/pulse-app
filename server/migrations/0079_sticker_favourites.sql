-- Sticker favourites (#498): starred GIPHY stickers, their own list beside favourite GIFs. They share gif_favourites
-- (a sticker's GIPHY id is never a GIF's), marked by `sticker`; each list has its own cap (checked by the route).
-- Every existing row is a GIF (0). ADD COLUMN is not a table rebuild, so foreign keys being OFF while migrating (db.rs)
-- changes nothing: no row is copied, dropped or cascaded.
-- Rollback = restore the pre-deploy backup (or leave it: an older server never reads `sticker`, but would list starred
-- stickers among the GIFs).
ALTER TABLE gif_favourites ADD COLUMN sticker INTEGER NOT NULL DEFAULT 0 CHECK (sticker IN (0, 1));
