-- A due reminder that fails to send counts its failures, so one bad row can't hold up every reminder behind it: the
-- sweep skips one that has failed `reminders::MAX_FAILURES` times (parked), and drops it with the fired ones after
-- the inbox's days. A new column with a default only: an older server ignores it.
-- Rollback = restore the pre-deploy backup (or leave it: an older server never reads or writes it).
ALTER TABLE message_reminders ADD COLUMN failures INTEGER NOT NULL DEFAULT 0;
