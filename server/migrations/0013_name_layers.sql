-- Name layers (#47, #48). A display name is how a person's name looks by default; it starts as a
-- copy of the username (registration sets it the same way), so nobody is left without one.
ALTER TABLE users ADD COLUMN display_name TEXT NOT NULL DEFAULT '';
UPDATE users SET display_name = username;

-- Names you gave people that only you see. Never sent to anyone but `owner_id`.
CREATE TABLE private_nicknames (
    owner_id   TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    target_id  TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    nickname   TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (owner_id, target_id)
) STRICT;
