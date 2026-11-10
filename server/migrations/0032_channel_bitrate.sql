-- Per-voice-channel bitrate (#75). One new nullable column on channels, nothing else: the audio bitrate in kbps that
-- everyone publishing in that voice channel sends at (the routes only store one of CHANNEL_BITRATES_KBPS, and only on
-- a voice channel). NULL = Default (the client's 64 kbps), which every existing row gets, so nothing changes until
-- someone picks a bitrate.
--
-- ADD COLUMN is not a table rebuild, so foreign keys being OFF while migrating (db.rs) changes nothing: no row is
-- copied, dropped or cascaded, and the column references nothing. It goes with its channel row (a purge).
-- Rollback = restore the pre-deploy backup (or ignore the column: older servers never read it).
ALTER TABLE channels ADD COLUMN bitrate INTEGER;
