-- Server delete (#184): its owner deleting a server moves it to their Recently deleted for 7 days
-- instead of purging it, as #112 does for channels. While `deleted_at` is set the server and
-- everything in it are invisible (`servers::routes::server_exists`, `is_server_member`,
-- `access::load_channel` and the Ready lists skip it) but nothing is removed, so a restore
-- (clearing both columns) brings it back whole. The sweeper purges it once 7 days have passed.
--
-- `deleted_by` is who deleted it (its owner, the only one who may). Users are never deleted, so
-- it keeps its referent; it's nullable only because a live server has none. Two added columns
-- and an index: no row changes and no table is rebuilt.
ALTER TABLE servers ADD COLUMN deleted_at TEXT;
ALTER TABLE servers ADD COLUMN deleted_by TEXT REFERENCES users(id);
-- The sweeper's scan: only deleted rows are indexed.
CREATE INDEX servers_deleted ON servers(deleted_at) WHERE deleted_at IS NOT NULL;
