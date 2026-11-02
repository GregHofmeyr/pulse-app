-- Link embeds (spec 2026-11-02-link-embeds-design): one preview per link of a message, built by
-- the server's unfurler after the send or an edit and sent out in the message's `embeds`.
--
-- Keyed by the link as it appears in the message, so an edit that keeps a link keeps its row
-- (and its `hidden`: a preview the sender removed stays removed through edits). `position` is the
-- link's place among the message's links, the order the previews show in.
-- `thumb_hash` is a `link_thumb` media row; the media cleanup keeps it while a row here
-- references it. A soft delete removes the message's rows (`embeds::on_delete`), and the cascade
-- covers a hard delete.
CREATE TABLE link_embeds (
    message_id   TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    position     INTEGER NOT NULL,
    url          TEXT NOT NULL,
    provider     TEXT NOT NULL,
    provider_id  TEXT,
    kind         TEXT NOT NULL,
    title        TEXT,
    description  TEXT,
    site_name    TEXT,
    author       TEXT,
    thumb_hash   TEXT,
    thumb_width  INTEGER,
    thumb_height INTEGER,
    start_secs   INTEGER,
    vertical     INTEGER NOT NULL DEFAULT 0,
    hidden       INTEGER NOT NULL DEFAULT 0,
    fetched_at   TEXT NOT NULL,
    PRIMARY KEY (message_id, url)
) STRICT;
-- For the media cleanup's "still referenced" check.
CREATE INDEX link_embeds_thumb ON link_embeds(thumb_hash) WHERE thumb_hash IS NOT NULL;
