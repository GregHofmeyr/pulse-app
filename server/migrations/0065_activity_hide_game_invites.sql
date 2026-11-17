-- Hide what I'm playing per person, and invites to play (#436). New tables only (no rebuilds), so foreign keys being
-- OFF while migrating (db.rs) changes nothing, and older servers never read them.
-- Rollback = restore the pre-deploy backup (or leave them: an older server ignores both, so hidden people would see
-- your game again and invites would show as their text).

-- People who never get your game activity (your "Playing", its Join, your Steam hours): they see you online with no
-- game. Yours only; they're never told. Goes with either person.
CREATE TABLE activity_hidden (
    user_id    TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    target_id  TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at TEXT NOT NULL,
    PRIMARY KEY (user_id, target_id)
) STRICT;

-- An invite to play: an ordinary message in a DM or group (its `content` is the invite as text, for clients that don't
-- know invites) with a row here. The game as the sender's "Playing" showed it when sent (`art`: its `GameArt` as
-- JSON). `join_info` is where to join (a `SteamJoin` as JSON), read from Steam at send time and on each Join round
-- while the invite lasts; it never leaves the server but to the chat's members, on request. `ended_at`: the sender
-- left the game or its lobby (or the server restarted), for good; `join_info` is cleared with it. Goes with its
-- message, its chat or its sender.
CREATE TABLE game_invites (
    message_id   TEXT PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
    channel_id   TEXT NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
    sender_id    TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    game         TEXT NOT NULL,
    steam_app_id INTEGER,
    art          TEXT,
    join_info    TEXT,
    ended_at     TEXT,
    created_at   TEXT NOT NULL
) STRICT;

-- The live invites of one sender (every game change and Join round reads them).
CREATE INDEX game_invites_live ON game_invites(sender_id) WHERE ended_at IS NULL;
