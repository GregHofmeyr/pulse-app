-- Group DM icons (#56): any member of a group may set or clear its icon. A media hash from the avatar upload
-- (POST /media/avatar); NULL = the members' faces, as before. Only groups use it. The media sweep keeps a hash in use
-- here (media::cleanup).
ALTER TABLE channels ADD COLUMN icon_hash TEXT;
