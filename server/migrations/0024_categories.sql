-- Channel categories (#54): named headings that hold a server's channels of any kind, replacing
-- the four fixed sections. A channel's `position` is now its order inside its category (or inside
-- the uncategorised area at the top, `category_id` NULL). `layout_revision` goes up with every
-- change to a server's layout, so a reorder based on an older one can be refused.
--
-- `default_kind` is the kind a default category was made for (here, or for a new server), so a
-- section someone had collapsed before categories maps onto it even after a rename. NULL for
-- categories people make.
ALTER TABLE servers ADD COLUMN layout_revision INTEGER NOT NULL DEFAULT 0;
CREATE TABLE categories (
    id           TEXT PRIMARY KEY,
    server_id    TEXT NOT NULL REFERENCES servers(id) ON DELETE CASCADE,
    name         TEXT NOT NULL,
    position     INTEGER NOT NULL,
    default_kind TEXT CHECK (default_kind IN ('text', 'quotes', 'clips', 'voice')),
    created_at   TEXT NOT NULL
) STRICT;
CREATE INDEX categories_server ON categories(server_id, position);
-- Deleting a category never deletes its channels: they lose the category.
ALTER TABLE channels ADD COLUMN category_id TEXT REFERENCES categories(id) ON DELETE SET NULL;
CREATE INDEX channels_category ON channels(category_id);

-- Nothing looks moved: each server gets a category per section it shows today (a kind with at
-- least one live channel), named as today's headings and in today's order.
--
-- A category's id is the smallest live channel id of its (server, kind): a valid ULID (so it
-- parses as a `CategoryId`), and unique among categories, since a channel id is the minimum of at
-- most one (server, kind) group. That also lets the channels find their category below.
INSERT INTO categories (id, server_id, name, position, default_kind, created_at)
SELECT first_id,
       server_id,
       CASE kind
           WHEN 'text' THEN 'Text Channels'
           WHEN 'quotes' THEN 'Quotes'
           WHEN 'clips' THEN 'Clips'
           ELSE 'Voice Channels'
       END,
       ROW_NUMBER() OVER (
           PARTITION BY server_id
           ORDER BY CASE kind WHEN 'text' THEN 0 WHEN 'quotes' THEN 1 WHEN 'clips' THEN 2 ELSE 3 END
       ) - 1,
       kind,
       strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
FROM (
    SELECT server_id, kind, MIN(id) AS first_id
    FROM channels
    WHERE server_id IS NOT NULL
      AND deleted_at IS NULL
      AND kind IN ('text', 'quotes', 'clips', 'voice')
    GROUP BY server_id, kind
);

-- Every channel of a kind with a category joins it, Recently deleted ones too (they keep their
-- category, #112). A deleted channel of a kind without one stays uncategorised: a restore puts
-- it at the bottom of the top area.
UPDATE channels
SET category_id = (
    SELECT cat.id FROM categories cat
    WHERE cat.server_id = channels.server_id AND cat.default_kind = channels.kind
)
WHERE server_id IS NOT NULL;

-- Live channels number 0..n inside their category, in today's sidebar order (position, then
-- name). Deleted ones keep their old position; a restore places them afresh.
UPDATE channels
SET position = ranked.pos
FROM (
    SELECT id,
           ROW_NUMBER() OVER (PARTITION BY server_id, kind ORDER BY position, name, id) - 1 AS pos
    FROM channels
    WHERE server_id IS NOT NULL AND deleted_at IS NULL
) AS ranked
WHERE channels.id = ranked.id;

-- Migrations run with foreign keys OFF (`db::connect`), so nothing above was checked as it ran:
-- any violation in `channels` or `categories` fails the migration, rolled back. Scoped on
-- purpose: an unrelated orphan row elsewhere must not stop the server booting.
CREATE TEMP TABLE migration_guard (ok INTEGER NOT NULL CHECK (ok = 0));
INSERT INTO migration_guard
    SELECT 1 FROM (
        SELECT 1 FROM pragma_foreign_key_check('channels')
        UNION ALL SELECT 1 FROM pragma_foreign_key_check('categories')
    ) LIMIT 1;
DROP TABLE migration_guard;
