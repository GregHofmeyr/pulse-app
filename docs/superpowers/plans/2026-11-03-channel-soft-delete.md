# Channel soft delete (Recently deleted) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Deleting a server channel (#104) moves it to a server's **Recently deleted** for 7 days, from where it can be
restored or purged, instead of purging it at once (#112).

**Architecture:** Two nullable columns on `channels` (`deleted_at`, `deleted_by`). `access::load_channel` and every
channel listing skip a deleted row, so the single access rule (`channel_for`), the event audience and the voice code all
treat it as gone with no per-route changes. The #104 purge becomes `servers::channels::purge`, called by the owner's
"Delete now" and by an hourly sweeper for channels deleted more than 7 days ago. Restore clears the columns and publishes
a new `ChannelRestored` event plus each member's absolute read counts and mutes.

**Tech Stack:** Rust (axum, sqlx/SQLite), ts-rs bindings, Tauri 2 commands, Svelte 5 + vitest.

**Spec:** the task brief recorded in issue #112 (owner's decision, 2026-11-03).

## Global Constraints

- Migration `server/migrations/0014_channel_soft_delete.sql` (0013 is taken by the parallel name-layers work; renumber to
  the next free number if main has moved).
- Retention: `RECENTLY_DELETED_DAYS = 7`.
- Who: anyone in the server deletes; the server owner (`servers.created_by`) or the deleter restores; only the owner
  purges ("Delete now").
- New protocol fields are `#[serde(default)]`; old clients ignore unknown events (they recover on reconnect).
- Dialog copy: "Delete {label}? It moves to Recently deleted for 7 days, where the server owner or you can restore it."
- Public repo: test people alex, sam, kim, jo only.

## Review Focus

1. A soft-deleted channel's messages reached through a message id (reactions, threads, edits, quote awards, clip routes)
   must 404 — covered by `load_channel` filtering; test: reacting to a message in a deleted channel is 404.
2. Restoring a voice channel nobody is in, and restoring while a same-named channel exists: both just work (names aren't
   unique); test: restore after creating a new channel of the same name keeps both.
3. A restore racing the sweeper / a purge: restore of a purged channel is 404, purge of a restored channel is 404 (the
   purge only touches rows with `deleted_at IS NOT NULL`); test: purge after restore is 404.
4. Server search with `in:` a deleted channel finds nothing (not a 404 leak); test in Task 2.
5. A quotes channel holding a QOTY winner never purges by time: it stays listed with `held` set; test in Task 3.

---

### Task 1: Soft delete, invisibility, migration

**Files:**
- Create: `server/migrations/0014_channel_soft_delete.sql`
- Modify: `server/src/access.rs`, `server/src/servers/routes.rs`, `server/src/servers/channels.rs`, `server/src/reads.rs`,
  `server/src/qoty/ending.rs` (nobody-voted cards), quote listings in `server/src/quotes/{routes,mod,awards,links}.rs`
  and `server/src/qoty/routes.rs` (server-wide `JOIN channels c` get `AND c.deleted_at IS NULL`)
- Test: `server/tests/channel_manage.rs`

**Interfaces:**
- Produces: `channels.deleted_at TEXT NULL`, `channels.deleted_by TEXT NULL REFERENCES users(id)`; `DELETE /channels/{id}`
  = soft delete (204), same `ChannelDeleted` event.

- [ ] Rewrite `deleting_a_text_channel_takes_everything_in_it` as `deleting_a_text_channel_moves_it_to_recently_deleted`:
  204, `ChannelDeleted` heard, gone from `/servers/{id}/channels`, Ready, server search (also with `in:`), messages list
  404, posting 404, reacting 404, a second delete 404; but the `channels` row, messages, reactions, mentions, search docs,
  read states and mute rows are all still there, with `deleted_by` = sam.
- [ ] Voice test: after delete nobody is in the call, LiveKit removals as before, and no `voice_sessions` row is open
  (`left_at IS NULL`); joining it is 404.
- [ ] Implement: migration; `load_channel`, `server_channels`, `all_server_channels`, `readable_server_channels` skip
  deleted; `delete` sets the columns and closes open voice sessions in one transaction (the QOTY guards move to Task 3).
  `read_states`/`thread_seen_for` skip deleted channels.
- [ ] Run `cargo test -p pulse-server --test channel_manage`; commit (`#112`).

### Task 2: Recently deleted list and restore

**Files:**
- Modify: `protocol/src/rest.rs` (`DeletedChannel`), `protocol/src/gateway.rs` (`Event::ChannelRestored`),
  `server/src/servers/channels.rs`, `server/src/dms/routes.rs` (router), `server/src/servers/routes.rs` (router)
- Test: `server/tests/channel_manage.rs`, `server/tests/gateway.rs` (audience row)

**Interfaces:**
- Produces:
  - `GET /servers/{id}/deleted-channels` → `Vec<DeletedChannel>` newest first; members only (403 / 404 like other
    server routes).
  - `DeletedChannel { channel: Channel, deleted_by: Option<UserId>, deleted_at: String, purge_at: String,
    can_restore: bool, can_purge: bool, #[serde(default)] held: Option<String> }` (`held`: why the sweeper keeps it).
  - `POST /channels/{id}/restore` → `Channel` (200); 404 unless it is a deleted server channel; 403 unless owner or
    deleter.
  - `Event::ChannelRestored { channel: Channel }`, scoped to the channel (so everyone, once visible again). Followed by a
    `ReadStateUpdated` (absolute counts) for each user with a read point in it and a `MutesChanged` for each user with a
    mute on it.

- [ ] Tests: `recently_deleted_lists_who_when_and_what_each_viewer_may_do` (alex owner, sam deleter, kim member: flags);
  `the_deleter_or_the_owner_restores_and_everyone_hears` (kim 403; sam 200; event; back in the list with its position and
  messages; sam's unread count comes back via `ReadStateUpdated`); `restoring_a_live_channel_is_404`.
- [ ] Implement; add the gateway audience case; `cargo test -p pulse-protocol export_bindings` to regenerate bindings.
- [ ] Run the tests; commit.

### Task 3: Purge ("Delete now") and the sweeper

**Files:**
- Modify: `server/src/servers/channels.rs` (`purge`, `purge_expired`, `spawn_sweeper`), `server/src/lib.rs`
- Test: `server/tests/channel_manage.rs`, `server/tests/qoty.rs`, `server/tests/clips.rs`

**Interfaces:**
- Produces: `DELETE /channels/{id}/permanent` → 204, owner only (403), 404 unless deleted; 409 with the existing QOTY
  messages. `pub async fn purge_expired(s: &AppState, now: DateTime<Utc>) -> AppResult<u32>` (channels purged);
  `spawn_sweeper` runs it at start and hourly.

- [ ] Tests: the old "takes everything in it" assertions move to `deleting_now_purges_everything` (owner only) and to
  `the_sweeper_purges_after_seven_days` (6 days: kept; 8 days: gone). QOTY: soft delete during a live vote / with a
  winner is now 204; Delete now is 409; the sweeper keeps both and lists `held`. Clips: the R2 object goes on purge, not
  on soft delete.
- [ ] Implement by moving the #104 body into `purge` (same transaction order, clip objects after commit).
- [ ] Run tests; commit.

### Task 4: Client

**Files:**
- Modify: `client/src-tauri/src/{api,commands,lib}.rs`, `client/ui/src/lib/tauri.ts`, `client/ui/src/lib/state.ts`
  (`ChannelRestored` = add the channel), `client/ui/src/lib/jump.ts` + `routes/Shell.svelte` (a jump into an unknown
  channel says "That channel was deleted."), `client/ui/src/components/DeleteChannelDialog.svelte` (copy),
  `client/ui/src/components/ServerSettings.svelte` + new `RecentlyDeleted.svelte`
- Test: `client/src-tauri/src/api.rs` test, `client/ui/src/components/RecentlyDeleted.test.ts`, `ChannelMenu.test.ts`,
  `lib/state.test.ts`

**Interfaces:**
- Consumes: Task 2/3 routes. Produces commands `list_deleted_channels(server_id)`, `restore_channel(channel_id)`,
  `purge_channel(channel_id)`; `api.listDeletedChannels / restoreChannel / purgeChannel`.

- [ ] Tests: list renders name, kind, "Deleted by sam", days left; Restore only when `can_restore`; Delete now only when
  `can_purge`, behind a confirm; `held` shown; restore removes the row. State: `ChannelRestored` adds the channel.
  Dialog copy. Jump to an unknown channel sets the notice.
- [ ] Implement; `pnpm check`, `pnpm test`, `cargo test -p pulse-client --lib`; commit.

### Task 5: Review, merge main, final checks

- [ ] One fresh reviewer pass; fix Critical/Important test-first.
- [ ] `git merge main`; renumber the migration if needed; fmt, clippy `-D warnings`, server + client tests, bindings no
  diff, `pnpm check`, `pnpm test`.
