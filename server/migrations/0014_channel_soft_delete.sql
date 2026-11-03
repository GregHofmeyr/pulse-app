-- Channel soft delete (#112): deleting a server channel moves it to its server's Recently deleted
-- for 7 days instead of purging it. While `deleted_at` is set the channel is invisible everywhere
-- (`access::load_channel` and the channel listings skip it) but everything in it is kept, so a
-- restore (clearing both columns) brings it back whole. The purge, by the owner or by the sweeper
-- once 7 days have passed, is the hard delete #104 introduced.
--
-- `deleted_by` is who deleted it: they may restore it, as may the server owner. Users are never
-- deleted, so it keeps its referent; it's nullable only because a live channel has none.
ALTER TABLE channels ADD COLUMN deleted_at TEXT;
ALTER TABLE channels ADD COLUMN deleted_by TEXT REFERENCES users(id);
-- The sweeper's scan: only deleted rows are indexed.
CREATE INDEX channels_deleted ON channels(deleted_at) WHERE deleted_at IS NOT NULL;
