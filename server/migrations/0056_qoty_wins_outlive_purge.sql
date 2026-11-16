-- A Quote of the Year win outlives its quote's channel being purged (#110). Until now `qoty_winners.message_id`
-- cascaded from `messages`, so a quotes channel holding a winner was held in Recently deleted for good. Now the win
-- keeps no foreign key on its message: a purge removes the quote (as a delete removes its words, spec §3.1) and the
-- win stays, read on the leaderboard as a deleted quote, with its vote count and its speaker snapshot
-- (`qoty_winner_speakers`) as before. `voters` is filled by the purge just before the quote's options and votes go
-- (a JSON array of user ids, in voting order); NULL means the votes are still there to read.
--
-- `qoty_winners` is rebuilt (a foreign key can't be dropped in SQLite), as 0008, 0009 and 0027 rebuilt theirs.
-- `db::connect` runs migrations with foreign keys OFF, so dropping it doesn't cascade into `qoty_winner_speakers`;
-- the guard below makes the migration fail, rolled back, if it is ever run with them on.
-- Rollback = restore the pre-deploy backup (an older server reads the rebuilt table as before, as long as no channel
-- holding a winner has been purged since).
CREATE TEMP TABLE migration_guard (ok INTEGER NOT NULL CHECK (ok = 0));
INSERT INTO migration_guard SELECT foreign_keys FROM pragma_foreign_keys;

CREATE TABLE qoty_winners_new (
    event_id   TEXT NOT NULL REFERENCES qoty_events(id) ON DELETE CASCADE,
    message_id TEXT NOT NULL,
    votes      INTEGER NOT NULL,
    voters     TEXT,
    PRIMARY KEY (event_id, message_id)
) STRICT;
INSERT INTO qoty_winners_new (event_id, message_id, votes)
    SELECT event_id, message_id, votes FROM qoty_winners;
DROP TABLE qoty_winners;
ALTER TABLE qoty_winners_new RENAME TO qoty_winners;
-- "Has this quote won?" (adding an option, the quote's wins, a reopen's leftovers).
CREATE INDEX qoty_winners_message ON qoty_winners (message_id);

-- Every copied row must still point at its event, and every speaker at its win: a violation fails the migration.
INSERT INTO migration_guard SELECT 1 FROM pragma_foreign_key_check('qoty_winners') LIMIT 1;
INSERT INTO migration_guard SELECT 1 FROM pragma_foreign_key_check('qoty_winner_speakers') LIMIT 1;
DROP TABLE migration_guard;
