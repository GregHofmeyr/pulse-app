-- Inbox reads by message (#327): the hygiene sweep prunes `inbox_reads` by message id (and a deleted message's
-- rows cascade by it), which the (user_id, message_id) key can't serve. One new index; nothing else changes.
-- Rollback = drop the index (older servers never name it).
CREATE INDEX inbox_reads_message ON inbox_reads(message_id);
