-- DMs/groups, unread, mutes, presence (spec 2026-10-08-dms-design).
ALTER TABLE notification_prefs ADD COLUMN muted_until TEXT;
ALTER TABLE users ADD COLUMN last_seen_at TEXT;

CREATE TABLE dm_hidden (
    user_id    TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    channel_id TEXT NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
    hidden_at  TEXT NOT NULL,
    PRIMARY KEY (user_id, channel_id)
) STRICT;

-- Start everyone at "all read": server text channels of servers they belong to, and their DMs/groups.
INSERT OR IGNORE INTO read_states (user_id, channel_id, last_read_message_id)
SELECT sm.user_id, c.id, (SELECT MAX(m.id) FROM messages m WHERE m.channel_id = c.id)
FROM server_members sm JOIN channels c ON c.server_id = sm.server_id AND c.kind = 'text';

INSERT OR IGNORE INTO read_states (user_id, channel_id, last_read_message_id)
SELECT cm.user_id, cm.channel_id, (SELECT MAX(m.id) FROM messages m WHERE m.channel_id = cm.channel_id)
FROM channel_members cm;

CREATE INDEX mentions_message ON mentions(message_id);
