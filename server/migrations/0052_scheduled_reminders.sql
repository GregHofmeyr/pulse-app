-- Scheduled messages (#299) and message reminders (#300). New tables only (no rebuilds), so foreign keys being OFF
-- while migrating changes nothing. Both are read by a sweep from the database (times are UTC, RFC 3339, milliseconds,
-- so they compare as strings), so a restart neither loses nor repeats one.
-- Rollback = restore the pre-deploy backup (or drop the two tables: older servers never read them; pending scheduled
-- messages and reminders are then lost, nothing else).

-- A message written now and posted by the server at `send_at`, as its author. Text only (mentions and emoji are
-- text): no files or GIFs. `failed` (with `failed_at`) is why it couldn't be posted when its time came (the author
-- left the chat, the chat was deleted, ...): kept, so its author sees it in their Scheduled list until they delete
-- it. Posted ones are deleted. Goes with its author or its chat.
CREATE TABLE scheduled_messages (
    id          TEXT PRIMARY KEY,
    author_id   TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    channel_id  TEXT NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
    content     TEXT NOT NULL,
    reply_to_id TEXT,
    send_at     TEXT NOT NULL,
    created_at  TEXT NOT NULL,
    edited_at   TEXT,
    failed_at   TEXT,
    failed      TEXT
) STRICT;

-- The sweep's look for what is due.
CREATE INDEX scheduled_messages_due ON scheduled_messages(send_at) WHERE failed_at IS NULL;
-- A person's list (and their cap).
CREATE INDEX scheduled_messages_author ON scheduled_messages(author_id, send_at);

-- "Remind me about this": private to `user_id`. `fired_at` is set once it has come due (it is then in their inbox,
-- unread until `seen_at`); done ones are deleted, and the sweep deletes fired ones after the inbox's 30 days. Goes
-- with its person or its message.
CREATE TABLE message_reminders (
    id         TEXT PRIMARY KEY,
    user_id    TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    message_id TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    remind_at  TEXT NOT NULL,
    created_at TEXT NOT NULL,
    fired_at   TEXT,
    seen_at    TEXT
) STRICT;

CREATE INDEX message_reminders_due ON message_reminders(remind_at) WHERE fired_at IS NULL;
CREATE INDEX message_reminders_user ON message_reminders(user_id, remind_at);
CREATE INDEX message_reminders_message ON message_reminders(message_id);
