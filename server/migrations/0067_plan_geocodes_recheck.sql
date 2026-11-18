-- Plans' geocoding (#451) used to take Open-Meteo's first result for any place, so free text like "test" was cached
-- as some far-away village. It now prefers the home country and needs the name to match; every cached answer goes,
-- so each place is asked again (lazily, one at a time) under the new rules.
-- Rollback = nothing to undo: an older server just fills the cache again its own way.
DELETE FROM plan_geocodes;
