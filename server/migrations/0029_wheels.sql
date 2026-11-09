-- The picker wheel (#158). New tables only (no rebuilds), so foreign keys being OFF while migrating changes nothing.
-- Rollback = restore the pre-deploy backup (or drop both tables; only spin lines' payloads mention them, and those
-- carry their own copy of the spin).

-- Saved wheels: a server wheel (`server_id`: everyone in the server sees and edits it) or a Just me wheel
-- (`owner_id`: only its owner). `options` is a JSON array of strings, in slice order.
CREATE TABLE wheels (
    id         TEXT PRIMARY KEY,
    server_id  TEXT REFERENCES servers(id) ON DELETE CASCADE,
    owner_id   TEXT REFERENCES users(id) ON DELETE CASCADE,
    name       TEXT NOT NULL,
    options    TEXT NOT NULL,
    created_by TEXT REFERENCES users(id) ON DELETE SET NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    CHECK ((server_id IS NULL) != (owner_id IS NULL))
) STRICT;

CREATE INDEX wheels_server ON wheels(server_id) WHERE server_id IS NOT NULL;
CREATE INDEX wheels_owner ON wheels(owner_id) WHERE owner_id IS NOT NULL;

-- Every spin, keyed by its system line: what was spun and where it landed (the server's draw), so Spin again and
-- Remove winner & spin run from the stored spin. Goes with its line or its chat.
CREATE TABLE wheel_spins (
    message_id TEXT PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
    channel_id TEXT NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
    spinner_id TEXT REFERENCES users(id) ON DELETE SET NULL,
    wheel_id   TEXT REFERENCES wheels(id) ON DELETE SET NULL,
    name       TEXT,
    options    TEXT NOT NULL,
    winner     INTEGER NOT NULL,
    angle      REAL NOT NULL,
    created_at TEXT NOT NULL
) STRICT;

CREATE INDEX wheel_spins_channel ON wheel_spins(channel_id);
CREATE INDEX wheel_spins_wheel ON wheel_spins(wheel_id) WHERE wheel_id IS NOT NULL;
