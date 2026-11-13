-- The profile card glow toggle (#156). One new column on users, nothing else: 1 = the card glows in its two colours
-- (what every card did before), 0 = the owner turned it off. Every existing row gets 1, so nothing changes until
-- someone turns it off. Saved with the profile (PUT /me/profile); a save without the field (an older client) keeps it.
--
-- ADD COLUMN is not a table rebuild, so foreign keys being OFF while migrating (db.rs) changes nothing: no row is
-- copied, dropped or cascaded, and the column references nothing. It goes with its user row.
-- Rollback = restore the pre-deploy backup (or ignore the column: older servers never read it).
ALTER TABLE users ADD COLUMN card_glow INTEGER NOT NULL DEFAULT 1;
