-- Notification levels per server and server channel (#195). NULL = the default: All messages for a server,
-- following its server for a channel. Existing rows (mutes) keep NULL, so nobody's alerts change until they pick.
ALTER TABLE notification_prefs ADD COLUMN level TEXT CHECK (level IN ('all', 'mentions', 'nothing'));
