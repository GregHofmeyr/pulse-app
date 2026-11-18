-- Servers are invite-only (#80, step 2): someone removed from a server is back only through a new invite, and the
-- owner's ban (0069's `server_bans`) refuses even that. The rejoin block (#418, 0064) is gone with the open Join, so
-- its two tables go. Plain drops of tables nothing references.
-- Rollback = restore the pre-deploy backup (an older server needs both tables to start removing people again).
DROP TABLE server_blocks;
DROP TABLE server_removals;
