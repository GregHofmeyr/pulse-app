-- Join sounds (#121). One new nullable column on server_members, nothing else: each member's join sound in that
-- server, a sound of its soundboard (the routes check it is that server's). NULL = none (the normal join cue), which
-- every existing row gets.
--
-- ADD COLUMN is not a table rebuild, so foreign keys being OFF while migrating (db.rs) changes nothing: no row is
-- copied, dropped or cascaded. At runtime (foreign keys ON) deleting the sound sets it back to NULL, and the column
-- goes with its member row (leaving the server, or the server's deletion).
-- Rollback = restore the pre-deploy backup (or ignore the column: older servers never read it).
ALTER TABLE server_members ADD COLUMN join_sound_id TEXT REFERENCES sounds(id) ON DELETE SET NULL;
