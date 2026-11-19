-- Your status in one server (#154): "My status here" on a server's right-click menu. Overrides your main status in
-- that server only (its members see it there; everyone sees your main status everywhere else, DMs included). A new
-- table only, so foreign keys being OFF while migrating (db.rs) changes nothing, and an older server ignores it.
-- Rollback = restore the pre-deploy backup (or leave it: an older server ignores it, so everyone sees your main status
-- everywhere again).

-- `set_at`: when it was set; an Invisible one freezes your last seen there at that moment, as Appear offline does.
CREATE TABLE server_status (
    user_id   TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    server_id TEXT NOT NULL REFERENCES servers(id) ON DELETE CASCADE,
    status    TEXT NOT NULL CHECK (status IN ('online', 'dnd', 'invisible')),
    set_at    TEXT NOT NULL,
    PRIMARY KEY (user_id, server_id)
) STRICT;
CREATE INDEX server_status_by_server ON server_status (server_id);
