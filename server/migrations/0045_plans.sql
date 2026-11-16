-- Plan something (#387): an invite card in the chat with RSVPs, a reminder an hour before, and a forecast for its
-- place and day. New tables only (no rebuilds), so foreign keys being OFF while migrating changes nothing.
-- Rollback = restore the pre-deploy backup (or drop the four tables: a plan's message keeps its plain-text fallback,
-- and a reminder line carries its own copy of the title and time).

-- A plan, keyed by its message (an ordinary message whose content is the plan as text, for clients that don't know
-- plans). `starts_at` is UTC (RFC 3339, milliseconds, so times compare as strings). `reminded_at` is set once the
-- reminder has gone (or when it was never due: a plan made, or moved, to less than an hour away), and cleared when
-- the time moves further out; the sweep reads it from here, so a restart neither loses nor repeats a reminder.
-- `cancelled_at`: cancelled by its creator (no RSVPs, no reminder). Goes with its message or its chat.
CREATE TABLE plans (
    message_id   TEXT PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
    channel_id   TEXT NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
    title        TEXT NOT NULL,
    starts_at    TEXT NOT NULL,
    place        TEXT,
    note         TEXT,
    reminded_at  TEXT,
    cancelled_at TEXT,
    edited_at    TEXT,
    created_at   TEXT NOT NULL
) STRICT;

-- The reminder sweep's look for plans whose reminder is due.
CREATE INDEX plans_remind_due ON plans(starts_at) WHERE reminded_at IS NULL AND cancelled_at IS NULL;

-- One answer per person per plan: 'going', 'maybe' or 'cant'. Taking it back deletes the row.
CREATE TABLE plan_rsvps (
    message_id TEXT NOT NULL REFERENCES plans(message_id) ON DELETE CASCADE,
    user_id    TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    rsvp       TEXT NOT NULL CHECK (rsvp IN ('going', 'maybe', 'cant')),
    at         TEXT NOT NULL,
    PRIMARY KEY (message_id, user_id)
) STRICT;

-- Open-Meteo's geocoding answers by place as typed (lower-cased, trimmed). `latitude` NULL = no such place (kept
-- too, so a typo isn't asked again and again). Refreshed after 30 days.
CREATE TABLE plan_geocodes (
    query      TEXT PRIMARY KEY,
    name       TEXT,
    latitude   REAL,
    longitude  REAL,
    fetched_at TEXT NOT NULL
) STRICT;

-- Open-Meteo's 16-day daily forecast for a spot (`lat,lon` at two decimals), as JSON the server reads. Refreshed
-- after three hours.
CREATE TABLE plan_forecasts (
    spot       TEXT PRIMARY KEY,
    body       TEXT NOT NULL,
    fetched_at TEXT NOT NULL
) STRICT;
