# Delete a server (Recently deleted) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.
> Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A server's owner can delete it (#184). It goes to the owner's **Recently deleted** for 7 days, where the
owner can restore it whole; after 7 days the sweeper purges it and everything in it.

**Spec:** issue #184 and the owner's decision comment (2026-11-11): only the server's owner deletes; confirm by typing
the server's name; a 7-day Recently deleted grace period like channel soft delete (#112), restorable by the owner.

**Architecture:** Two nullable columns on `servers` (`deleted_at`, `deleted_by`), as #112 put on `channels`. The access
choke points learn that a deleted server is gone, so every route behind them treats it as missing with no per-route
change:

- `access::load_channel` skips a channel whose server is deleted: every channel-scoped route (messages, reactions,
  threads, pins, polls, wheels spun in chat, clips, voice tokens, LiveKit webhooks, streams, stage shares, soundboard
  plays, search `in:`) answers 404, and the gateway audience of any event scoped to one of its channels is nobody.
- `servers::routes::server_exists` (and so `require_member`) and `is_server_member` treat a deleted server as missing /
  not joined: server-scoped routes (join, channels, members, categories, emoji, soundboard, join sounds, wheels, speaker
  links, Quote of the Year, nicknames, server mute, Recently deleted channels, storage) answer 404 or 403.
- The listings that don't go through those skip it: `all_servers`, `all_server_channels`, `all_server_members`,
  `all_layouts`, the Ready lists of sounds, games, speaker links, Quote of the Year votes and server mutes, and the read
  state / thread-seen queries in `reads.rs`. Custom emoji stay in Ready (deleted emoji already do, so a message
  elsewhere that used one still renders); the pickers only offer emoji and sounds of servers you're a member of, and a
  deleted server has no members in Ready.

Nothing is purged or rewritten at delete time: a restore clears the two columns and everything is as it was. "Frozen"
falls out of this: nothing can reach the server's channels, messages, voice, streams, stage shares, wheels, soundboard,
emoji, join sounds or badge-counted activity, so nothing changes. The sweepers that would act on it by time skip it:
the channel sweeper leaves its Recently deleted channels alone (their own 7 days keep counting), and the Quote of the
Year sweeper doesn't end its vote (an overdue vote ends on the first sweep after a restore). Timed pins and polls run
out by their own clock as before (nobody sees it happen).

**Tech Stack:** Rust (axum, sqlx/SQLite), ts-rs bindings, Tauri 2 commands, Svelte 5 + vitest.

## Decisions

- **Who:** only the owner (`servers.created_by`) deletes and restores. Everyone else gets 403 (404 when there's no such
  live server). There is no "Delete now" for a server (the decision lists Restore only); see the follow-up issue.
- **Confirm:** `DELETE /servers/{id}` with `{ "name": "…" }`; the trimmed name must equal the server's name exactly,
  else 400 "Type the server's name to delete it." The client's Delete button stays disabled until it matches.
- **Delete:** one write transaction sets `deleted_at`/`deleted_by` and closes every open `voice_sessions` row in its
  channels. Then, under `in_order`, `ChannelDeleted` for each of its live channels (what 0.2.x clients understand:
  they drop each channel with its voice presence and streams) and the new `ServerDeleted { server_id }` (everyone:
  servers are public). Then, as a channel delete does for each voice channel: everyone in its calls is taken out of
  voice state, their streams end (badges told), viewers and ghost viewers dropped, its stage shares cleared, and
  LiveKit closes the calls in the background (`voice::revoke::close_call`). That part of `channels::delete` becomes a
  shared helper.
- **Invites:** Pulse has no per-server invites (invites are account sign-up codes). The way into a server is
  `POST /servers/{id}/join`, which 404s for a deleted server.
- **Owner's cap:** a deleted server still counts toward `SERVERS_PER_USER` until purged, as a deleted channel counts
  toward the channel cap, so a restore never takes the owner past it.
- **Recently deleted (servers):** `GET /servers/deleted` lists the asker's own deleted servers, newest first:
  `DeletedServer { server, deleted_at, purge_at, can_restore }` (`can_restore` false once due: it's being purged). The
  client shows it in Settings → Recently deleted (a deleted server has no Server settings to hold it: it's off the rail),
  each with its time left and Restore, matching the channels' list in Server settings.
- **Restore:** `POST /servers/{id}/restore` → `Server`. 404 unless it's deleted and not yet due, 403 unless the owner.
  The restore and the sweeper can't overlap: restore only matches `deleted_at > now − 7 days`, the sweeper only
  `deleted_at <= now − 7 days`. Then the new `ServerRestored { snapshot }` (everyone), carrying what Ready would have
  for it: the server, its channels, members, layout, custom emoji, sounds, games, speaker links, Quote of the Year
  vote and server mutes. Then each member's own state comes back as for a restored channel: `ReadStateUpdated` for
  each read point in its channels, `MutesChanged`/`NotifyLevelsChanged` for whoever muted it or one of its channels,
  `ClipViewChanged` for clip views.
- **Purge:** `servers::deleted::purge_expired(now)` from an hourly sweeper (started next to the channel sweeper).
  Batched, for #108's write-lock concern: per channel, messages go newest first in batches of 500 (a reply or thread
  reply is normally newer than what it points at; one that isn't, after a clock went back across a restart, has its
  link cleared rather than stopping the purge), each batch its
  own short `begin_write` transaction under `in_order`, quote lines before their messages (the search triggers), and
  each batch's clip objects removed from R2 after its commit (the clip sweeper retries a failure). Then one last
  transaction deletes its notification prefs (server and channel targets; no FK), its channels and the server row,
  whose cascades take categories, members (and join sounds), custom emoji, sounds (favourites, usage), games, wheels,
  speaker links, Quote of the Year votes and winners, server mutes and nickname history. Disk media (emoji, sounds,
  quote pictures, channel icons) is left to the hourly media cleanup once nothing uses it, as for every other delete.
  Servers have no icon of their own yet (`servers.icon_hash` is never set). A purge never stops for a channel `held`
  by a Quote of the Year winner: the whole server is going. Nobody hears of a purge (the server left everyone's view
  when it was deleted).
- **0.2.x clients:** unknown gateway frames are skipped (`client/src-tauri/src/gateway.rs` `next_frame`), so
  `ServerDeleted`/`ServerRestored` are ignored. On a delete they get a `ChannelDeleted` per channel: the channels,
  voice and streams go and an open channel steps back to its (now empty) server; the server's tab stays, empty, until
  their next Ready, which leaves it out. A restore shows up on their next Ready. The new `Server.owner_id` is optional
  and skipped when absent.

## Migration 0033 (`server/migrations/0033_server_soft_delete.sql`)

Adds two nullable columns to `servers` and one partial index; no rows change, nothing is rebuilt, so foreign keys being
off during migrations (`db.rs`) don't matter:

```sql
ALTER TABLE servers ADD COLUMN deleted_at TEXT;
ALTER TABLE servers ADD COLUMN deleted_by TEXT REFERENCES users(id);
CREATE INDEX servers_deleted ON servers(deleted_at) WHERE deleted_at IS NOT NULL;
```

## Review Focus

1. Every way in: a deleted server's channel by id (messages, voice token, stream, stage, search `in:`), its
   server-scoped routes (join, emoji, sounds, wheels, members, categories), and Ready all treat it as gone.
2. Restore vs. purge race (time windows above); restore of a purged server 404.
3. The batched purge leaves no orphans and no dangling `reply_to_id`/`thread_root_id`, and removes R2 objects.
4. Old clients: nothing they get on a delete breaks them.

---

### Task 1: Migration, protocol, invisibility

**Files:** create `server/migrations/0033_server_soft_delete.sql`, `server/src/servers/deleted.rs`; modify
`protocol/src/rest.rs` (`Server.owner_id`, `DeleteServerRequest`, `DeletedServer`), `protocol/src/gateway.rs`
(`ServerDeleted`, `ServerRestored`, `ServerSnapshot`), `server/src/access.rs`, `server/src/servers/routes.rs`,
`server/src/servers/categories.rs`, `server/src/reads.rs`, `server/src/{sounds,clips/games,quotes/links,qoty,voice/moderation}`
Ready lists, `server/src/qoty/ending.rs`, `server/src/servers/channels.rs` (sweeper, `deleted_channel`).
Test: `server/tests/server_delete.rs`.

- [ ] Tests: non-owner 403, wrong name 400, missing server 404; owner 204; `ChannelDeleted` per channel then
  `ServerDeleted` heard; gone from `/servers`, Ready (servers, channels, members, layouts, sounds, read states); its
  channels 404 (messages, post), join 404, emoji/sounds routes refused; a second delete 404; rows all still there.
- [ ] Implement; commit (`#184`).

### Task 2: Voice and streams end

- [ ] Test: alex and sam in a voice channel of the server → after delete nobody is in it, no open `voice_sessions`,
  LiveKit removals, voice token 404.
- [ ] Implement by moving the voice block of `channels::delete` into a shared helper; commit.

### Task 3: Recently deleted list and restore

- [ ] Tests: list shows only the owner's own deleted servers with `purge_at`; restore by non-owner 403; restore of a
  live server 404; owner restore 200, `ServerRestored` snapshot heard with channels and members, back in `/servers` and
  Ready with its messages, sam's read state republished.
- [ ] Implement; commit.

### Task 4: Purge and sweeper

- [ ] Tests: 6 days kept, 8 days gone; every row of the server gone (channels, messages incl. replies across batches,
  members, emoji, sounds, categories, prefs); other servers untouched; clip object removed from R2; restore after
  due 404.
- [ ] Implement; commit.

### Task 5: Client

**Files:** `client/src-tauri/src/{api,commands,lib}.rs`, `client/ui/src/lib/tauri.ts`, `client/ui/src/lib/state.ts`
(`ServerDeleted` drops it and everything of it; `ServerRestored` adds the snapshot), `routes/Shell.svelte` (the open
server is deleted: back to Home, "{name} was deleted."), `components/ServerSettings.svelte` (owner-only "Delete server"
section) + new `DeleteServer.svelte`, `components/Settings.svelte` + new `DeletedServers.svelte`.

- [ ] Tests: state reducers; DeleteServer button disabled until the name matches, calls the API, shows errors; only the
  owner sees the section; DeletedServers lists time left and Restore removes the row.
- [ ] Implement; `pnpm check`, `pnpm test`, client Rust tests, xwin clippy; commit.

### Task 6: Review, rebase, final checks, merge

- [ ] Code review of the diff; fix what's real test-first.
- [ ] Rebase on `origin/main`, renumber the migration if taken; fmt, clippy `-D warnings`, server + client tests,
  bindings no diff, `pnpm check`, `pnpm test`; merge to main with `Closes #184`; push.
