-- Server rail order (#185): the order each person dragged the servers into, theirs only, on every device.
-- Additive: a new table only (no rebuilds, so foreign keys being OFF while migrating changes nothing). Nobody has an
-- order yet, so everyone's rail stays as it is until they drag. A server missing from someone's rows (new since, or a
-- soft-deleted one coming back) just follows the ordered ones. Rollback = leave it (older servers never read it) or
-- `DROP TABLE server_order`.
CREATE TABLE server_order (
    user_id   TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    server_id TEXT NOT NULL REFERENCES servers(id) ON DELETE CASCADE,
    position  INTEGER NOT NULL,
    PRIMARY KEY (user_id, server_id)
) STRICT;
