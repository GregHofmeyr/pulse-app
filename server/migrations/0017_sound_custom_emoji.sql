-- A sound's emoji can be a server's custom emoji (#122). No foreign key: custom emoji are soft-deleted, and a sound
-- whose emoji went just shows without one.
ALTER TABLE sounds ADD COLUMN emoji_id TEXT;
