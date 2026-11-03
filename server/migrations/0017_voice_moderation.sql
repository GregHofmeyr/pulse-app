-- Server mute / server deafen (#140): per (server, member), who did each (NULL = not in force).
-- Lasts across leaving and rejoining until someone undoes it; the row goes once both are NULL.
CREATE TABLE voice_moderation (
    server_id   TEXT NOT NULL REFERENCES servers(id) ON DELETE CASCADE,
    user_id     TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    muted_by    TEXT REFERENCES users(id) ON DELETE SET NULL,
    deafened_by TEXT REFERENCES users(id) ON DELETE SET NULL,
    updated_at  TEXT NOT NULL,
    PRIMARY KEY (server_id, user_id)
) STRICT;
