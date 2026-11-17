-- Invites to play survive a server restart (#438). A restart still ends every open invite (what anyone plays lives
-- in memory), but marks it with `restart_grace_until` (startup + 10 minutes): when its sender reports the same game
-- again before then, it opens again. A new nullable column only (no rebuild), so an older server ignores it.
-- Rollback = restore the pre-deploy backup (or leave it: an older server never reads it, and invites a restart ended
-- simply stay ended, as before).
ALTER TABLE game_invites ADD COLUMN restart_grace_until TEXT;
