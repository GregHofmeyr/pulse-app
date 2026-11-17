-- Picker wheels' weighted options and palettes (#259). New nullable columns only (no rebuilds), so foreign keys being
-- OFF while migrating changes nothing, and an older server reads the tables as before (it ignores the columns).
-- Rollback = restore the pre-deploy backup (or leave the columns: nothing else reads them).
--
-- `weights`: a JSON array of whole numbers, one per option in `options` (1 to 10); NULL is every option alike.
-- `palette`: a palette's name (the client draws it); NULL is the neon spectrum.
ALTER TABLE wheels ADD COLUMN weights TEXT;
ALTER TABLE wheels ADD COLUMN palette TEXT;
-- A spin keeps what it was spun with, so Spin again and Remove winner & spin draw the same wheel.
ALTER TABLE wheel_spins ADD COLUMN weights TEXT;
ALTER TABLE wheel_spins ADD COLUMN palette TEXT;
