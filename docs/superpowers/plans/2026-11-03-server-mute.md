# Server mute, server deafen and disconnect Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Anyone in a server can server-mute, server-deafen or disconnect anyone else in that server's voice channels (and undo the mute/deafen), like Discord (#140).

**Architecture:** Server mute/deafen is a row per (server, user) in a new `voice_moderation` table, carried in `Ready` and a `ServerVoiceChanged` gateway event. The server enforces it at LiveKit: the voice token is minted with `can_publish: false`, a live participant gets `UpdateParticipant` with `can_publish: false` (LiveKit drops their mic track), and every join webhook reconciles the permission (covers prefetched tokens). Deafen implies mute; the hearing half of deafen is the client muting all remote audio (`Controls::playout_on`). Disconnect removes the participant from the voice and stream rooms (the `close_call` path) with no lasting state, after telling the target who did it (`VoiceDisconnected`, target only); LiveKit's `ParticipantRemoved` already stops the client's auto-rejoin.

**Tech Stack:** Rust (axum, sqlx/SQLite, livekit-api RoomClient), Tauri 2 client core (livekit 0.9.7), Svelte 5 UI, ts-rs bindings.

**Spec:** GitHub issue #140 plus the owner's brief (behaviour section) quoted in the session; this plan restates every decision.

## Global Constraints

- Not available in DM/group calls (no server); not on yourself, including undoing your own (another person must, like Discord).
- New protocol fields `#[serde(default)]`. Bindings regenerated with `cargo test -p pulse-protocol export_bindings`; no diff afterwards.
- Migration: next free number (`0017_voice_moderation.sql`); renumber at `git merge main` if main took 0017.
- UI colours: existing tokens only (`--danger`, `--on-danger`, `--danger-text`, `--focus-ring`); any new colour goes in every theme block.
- Public repo: test people alex/sam/kim/jo only.
- Builds: `CARGO_BUILD_JOBS=4 flock ~/Projects/pulse-app/target/.cargo-serial nice -n 19 ionice -c3 cargo …`, tests `-- --test-threads=4`, own worktree `target/`.
- The Doos (muted-talk) counts self-mute only: never while server-muted or server-deafened.

## Review Focus

1. A prefetched token minted before the mute (or while muted, used after the undo) — join webhook must reconcile LiveKit permission both ways. Test: `join_reconciles_publish_permission` (Task 2).
2. The target is in a voice channel of a *different* server — mute in server A must not touch their LiveKit permission in server B's room. Test: `mute_in_other_server_leaves_live_call_alone` (Task 2).
3. Undo while the target is offline/not in voice — must clear the row (so it doesn't greet them later) without calling LiveKit. Test: `undo_without_call_clears_row` (Task 1).
4. Deafen then unmute-only: deafen implies mute, so `mute:false` while deafened leaves publishing revoked. Test: `can_publish_follows_mute_or_deafen` (Task 1) + `Controls` test (Task 3).
5. Client joins while server-muted: the initial mic publish must be skipped, not treated as a failed publish that ends the call; and the mic republished when permission returns. Tests: `publish_allowed_*` (Task 3).

---

### Task 1: Protocol + table + mute/deafen route

**Files:**
- Create: `server/migrations/0017_voice_moderation.sql`, `server/src/voice/moderation.rs`, `server/tests/server_mute.rs`
- Modify: `protocol/src/rest.rs`, `protocol/src/gateway.rs` (Ready, Event, `channel_id`/`only_for`), `protocol/src/lib.rs` (export request), `server/src/voice/mod.rs`, `server/src/voice/routes.rs` (router), `server/src/gateway/socket.rs` (Ready)

**Interfaces (produces):**
- `rest::ServerVoiceMod { server_id: ServerId, user_id: UserId, muted_by: Option<UserId>, deafened_by: Option<UserId> }`
- `rest::SetServerVoiceRequest { #[serde(default)] mute: Option<bool>, #[serde(default)] deafen: Option<bool> }`
- `Ready.server_voice: Vec<ServerVoiceMod>` (`serde(default)`); `Event::ServerVoiceChanged { state: ServerVoiceMod }` (audience Everyone, like other server-level data); `Event::VoiceDisconnected { channel_id, user_id, by: UserId }` (`only_for` = `user_id`).
- `moderation::get(db, server, user) -> AppResult<Option<ServerVoiceMod>>`, `moderation::for_ready(db) -> AppResult<Vec<ServerVoiceMod>>`, `moderation::publish_allowed(m: Option<&ServerVoiceMod>) -> bool` (false when muted_by or deafened_by set).
- Route `POST /servers/{server}/voice/{user}` body `SetServerVoiceRequest` → 200 `ServerVoiceMod`.

Table: `voice_moderation(server_id REFERENCES servers ON DELETE CASCADE, user_id REFERENCES users, muted_by, deafened_by, updated_at, PK(server_id,user_id)) STRICT`; the row is deleted when both are null.

Rules: caller must be a member (`require_member`: 404/403); target == caller → 400; target not a member → 404; setting either to `true` needs the target in a voice channel of this server (`VoiceState::room_flags` → channel's `server_id`) else 409; undo needs nothing. A field that is `None` stays as it is; setting an already-set flag keeps its original `muted_by`. Publish `ServerVoiceChanged` only when the row changed.

- [ ] Step 1: tests in `server/tests/server_mute.rs`: `mute_sets_row_and_broadcasts` (alex mutes sam who is in Lounge → 200, `muted_by == alex`, sam's socket gets `ServerVoiceChanged`), `cannot_target_yourself` (400, also for undo), `outsider_gets_403`, `mute_needs_target_in_this_servers_voice` (409), `undo_without_call_clears_row` (row gone, `muted_by None` broadcast), `ready_carries_server_voice`, and unit test `can_publish_follows_mute_or_deafen` in moderation.rs. Run → compile failure (types missing).
- [ ] Step 2: implement protocol types, migration, module, route, Ready field. Regenerate bindings.
- [ ] Step 3: run `cargo t -p pulse-server --test server_mute` → PASS. Commit `feat(voice): server mute/deafen state and route (#140)`.

### Task 2: LiveKit enforcement + disconnect

**Files:** Modify `server/src/voice/moderation.rs`, `server/src/voice/routes.rs` (token grant + join webhook), `server/src/voice/revoke.rs` (shared `client`), `server/tests/common/livekit.rs` (fake `UpdateParticipant` with a log), `server/tests/server_mute.rs`.

**Interfaces:**
- `moderation::permission(allowed: bool) -> ParticipantPermission` (`can_subscribe: true`, `can_publish: allowed`, `can_publish_data: false`, `can_publish_sources: [Microphone]`).
- `moderation::enforce(s, channel, user, allowed)` — background-safe, logs failures, bounded by `revoke::BUDGET`.
- Route `POST /servers/{server}/voice/{user}/disconnect` → 204. Same member/self rules; target must be in this server's voice (404 otherwise). Publishes `VoiceDisconnected { by }` to the target, then `revoke::close_call(s, channel, &[target])`.
- Fake: `fake_livekit_logged` also returns `Updated = Arc<Mutex<Vec<(String, String, bool /*can_publish*/)>>>` via a new `fake_livekit_full(rooms) -> (url, Removed, Updated)`.

- [ ] Step 1: tests: `mute_revokes_publish_at_livekit` (fake sees `UpdateParticipant(room, sam, false)`; undo → `true`), `deafen_implies_no_publish`, `mute_in_other_server_leaves_live_call_alone`, `token_while_muted_cannot_publish` (JWT `can_publish == Some(false)`; still `true` otherwise), `join_reconciles_publish_permission` (webhook join with `permission.can_publish=true` while muted → Update false; with `false` while not muted → Update true; matching → no call), `disconnect_removes_from_voice_and_stream_rooms_and_tells_target` (Removed has `(lounge, sam)`, `(lounge~s, sam#v)`, `(lounge~s, sam#a)`; sam gets `VoiceDisconnected{by: alex}`; no row written), `disconnect_needs_target_in_voice` (404), `disconnect_self_400`. Run → FAIL.
- [ ] Step 2: implement. Token: `can_publish: Some(publish_allowed(..))` for server channels. Webhook `participant_joined` (after admit): if server channel, compare `p.permission.can_publish` with desired; spawn `enforce` on mismatch.
- [ ] Step 3: run tests → PASS; full `cargo t -p pulse-server` → PASS. Commit `feat(voice): enforce server mute at LiveKit; disconnect (#140)`.

### Task 3: Client core

**Files:** Modify `client/src-tauri/src/voice/controls.rs`, `client/src-tauri/src/voice/processor.rs` (muted-talk input), `client/src-tauri/src/voice/mod.rs` (publish skip/republish, `set_server`), `client/src-tauri/src/commands.rs` + `lib.rs` (command `set_server_voice`).

**Interfaces:**
- `Controls { pub server_muted: bool, pub server_deafened: bool }` (serialised). `set_server(muted, deafened)`. `mic_open()` false if any of muted/deafened/server_muted/server_deafened; `playout_on()` false if deafened or server_deafened; `counts_muted_talk() -> bool` = (muted||deafened) && !server_muted && !server_deafened. Self toggles unchanged (self state only).
- `voice::publish_allowed(p: Option<&ParticipantPermission>) -> bool` (None → true).
- `VoiceManager::set_server(muted, deafened) -> Controls` emits a `State` event.
- Tauri `set_server_voice(muted: bool, deafened: bool) -> Controls`.
- Room loop: initial publish skipped (reported OK) when `!publish_allowed(local permission)`; on `ParticipantPermissionChanged` for the local participant going to allowed: locally unpublish any stale mic publication, then publish the kept `LocalAudioTrack` again with `mic_publish_options()`.
- Processor feeds `MutedTalk::block(rms, c.counts_muted_talk(), now)`.

- [ ] Step 1: tests in controls.rs: `server_mute_closes_the_mic_but_keeps_self_state`, `server_deafen_stops_playout_and_mic`, `muted_talk_counts_self_mute_only`; in mod.rs: `publish_allowed_without_permission_info`, `publish_allowed_follows_can_publish`. Run → FAIL.
- [ ] Step 2: implement. Step 3: `cargo t -p pulse-client --lib` → PASS. Commit `feat(voice): client honours server mute/deafen (#140)`.

### Task 4: UI

**Files:** Modify `client/ui/src/lib/state.ts` (+test), `lib/selectors.ts` (+test), `lib/participantMenu.ts` (+test), `components/ParticipantMenu.svelte` (+test), `components/ChannelList.svelte`, `components/UserTile.svelte`, `components/VoiceStatus.svelte` (+test), `components/VoicePanel.svelte`, `routes/Shell.svelte`, `lib/voice.svelte.ts`, `lib/voiceui.ts` (Flags type), `lib/tauri.ts` (api calls).

**Interfaces:**
- `AppState.serverVoice: Record<serverId, Record<userId, ServerVoiceMod>>`, from Ready and `ServerVoiceChanged` (entry dropped when both null).
- `Occupant` gains `serverMutedBy: string | null`, `serverDeafenedBy: string | null`.
- `MenuCtx` gains `server: boolean` (server voice channel) and `serverMuted`, `serverDeafened`; non-self items in a server add `'server-mute' | 'server-deafen' | 'disconnect'` (after `message`); self/DM: none.
- `api.setServerVoice(serverId, userId, {mute?, deafen?})`, `api.disconnectVoice(serverId, userId)`.
- `voice.disconnectedBy: { by: string; channelId: string } | null` set from `VoiceDisconnected`, cleared on next join/dismiss.
- Effect in voice.svelte.ts: my entry for the current call's server → `invoke('set_server_voice', …)` on change (false/false when not in a server call).
- Copy: menu "Server mute"/"Server unmute", "Server deafen"/"Server undeafen", "Disconnect"; flags tooltip "Server muted by {name}" / "Server deafened by {name}" (solid red badge `.flag.server` = `--danger` bg / `--on-danger` icon); target notice in VoiceStatus "You were server muted by {name}" / "server deafened by {name}"; disconnect notice "{name} disconnected you from {channel}"; own mute button tip "Server muted by {name}".

- [ ] Step 1: tests: state reducer (`ServerVoiceChanged` set/clear, Ready), selectors (occupant carries by-ids), participantMenu (server items for others in server calls only, none for self or DMs), ParticipantMenu (labels flip, calls api), VoiceStatus (notice text for target). Run `pnpm test` → FAIL.
- [ ] Step 2: implement. Step 3: `pnpm check` 0/0, `pnpm test` PASS. Commit `feat(ui): server mute/deafen/disconnect menu, flags and notices (#140)`.

### Task 5: Review, merge, final checks

- [ ] One fresh reviewer pass (superpowers:requesting-code-review); fix Critical/Important test-first.
- [ ] `git merge main`; renumber migration if needed.
- [ ] fmt, clippy `--workspace --all-targets -D warnings`, `cargo test -p pulse-server`, `cargo test -p pulse-client --lib`, bindings no diff, `pnpm check`, `pnpm test`. Final commit `Closes #140`.
- [ ] Deferred items → GitHub issues.
