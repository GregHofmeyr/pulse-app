-- Purging big channels (#108). Deleting a message makes SQLite look for the rows whose foreign keys point at it, and
-- without an index on the child column that is a scan of the whole child table for every deleted message. Two
-- such columns had none: `messages.reply_to_id` (a scan of every message in the database per deleted message:
-- purging a 50k-message channel took minutes and held the write lock for seconds per batch) and
-- `thread_reads.root_id` (its key leads with user_id). Two new indexes; nothing else changes, so foreign keys being
-- OFF while migrating (db.rs) changes nothing.
-- Rollback = drop the indexes (older servers never name them).
CREATE INDEX messages_reply_to ON messages(reply_to_id) WHERE reply_to_id IS NOT NULL;
CREATE INDEX thread_reads_root ON thread_reads(root_id);
