-- "Notify me when they go live" (#60): the people `owner_id` wants a notification for when they go live.
-- Per account, so it follows you to every device; sent only to its owner. `created_at` keeps the list in
-- the order you added people.
CREATE TABLE live_notify (
    owner_id   TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    target_id  TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at TEXT NOT NULL,
    PRIMARY KEY (owner_id, target_id)
) STRICT;
