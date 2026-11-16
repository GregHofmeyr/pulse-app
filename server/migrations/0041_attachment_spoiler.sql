-- Spoiler attachments (#289): the sender marks a file as a spoiler, and it shows blurred until clicked.
-- Additive: a new column with a constant default (no table rebuild, so foreign keys being OFF while migrating changes
-- nothing). Files sent before this aren't spoilers. Rollback = leave it (older servers never name it) or
-- `ALTER TABLE attachments DROP COLUMN spoiler`.
ALTER TABLE attachments ADD COLUMN spoiler INTEGER NOT NULL DEFAULT 0 CHECK (spoiler IN (0, 1));
