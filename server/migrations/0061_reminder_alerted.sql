-- A reminder that comes due while none of its person's devices is connected is told on their next launch (#412),
-- once, by whichever device claims it first. `alerted_at` is when a device told them: set as it fires when one is
-- connected (they all hear it live), or by the first device to claim it after (`POST /reminders/{id}/alerted`).
-- Fired, unseen and still NULL = missed. One ADD COLUMN, no rebuild, so foreign keys being OFF while migrating
-- (db.rs) changes nothing.
-- Rollback = restore the pre-deploy backup (or ignore the column: older servers never read it).
ALTER TABLE message_reminders ADD COLUMN alerted_at TEXT;
-- Those already come due were told (or not) by the rules before this: none is told again.
UPDATE message_reminders SET alerted_at = fired_at WHERE fired_at IS NOT NULL;
