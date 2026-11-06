-- Voice channel chat (#226): a voice channel now holds messages, counted like a text channel's. Everyone already in a
-- server gets a read point in its voice channels, at the newest message there, as joining or a new channel gives one
-- (a channel without a point never counts anything). Deleted ones too: a restore brings their counts back.
INSERT OR IGNORE INTO read_states (user_id, channel_id, last_read_message_id)
SELECT sm.user_id, c.id, (SELECT MAX(m.id) FROM messages m WHERE m.channel_id = c.id)
FROM channels c
JOIN server_members sm ON sm.server_id = c.server_id
WHERE c.kind = 'voice';
