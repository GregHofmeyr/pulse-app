-- Plan something follow-ups (#399): an optional end time on a plan, and a per-person "Plan reminders" setting that
-- follows you to every device. Two ADD COLUMNs, no rebuilds, so foreign keys being OFF while migrating (db.rs) changes
-- nothing: no row is copied, dropped or cascaded, and neither column references anything.
-- Rollback = restore the pre-deploy backup (or ignore the columns: older servers never read them; a plan's end lives
-- on only in its card).

-- When a plan ends (UTC, RFC 3339, milliseconds, as `starts_at`); NULL = no end given (a calendar gets an hour).
ALTER TABLE plans ADD COLUMN ends_at TEXT;

-- 1 = plans' reminder lines mention you (what everyone had before), 0 = you turned Plan reminders off. Every existing
-- row gets 1, so nothing changes until someone turns it off.
ALTER TABLE users ADD COLUMN plan_reminders INTEGER NOT NULL DEFAULT 1 CHECK (plan_reminders IN (0, 1));
