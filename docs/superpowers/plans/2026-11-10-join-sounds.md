# Custom join sounds (#121) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** pick a sound from a server's soundboard as your join sound there; when you join that server's voice
channels, everyone in the call (you too) hears it instead of the normal join cue.

**Architecture:** the choice is a nullable column on `server_members` (so it goes with the membership and with the
sound: `ON DELETE SET NULL`), surfaced as `Member.join_sound` (Ready's member lists, `MemberUpdated` on change). The
LiveKit join webhook adds `join_sound` (hash, volume, length) to `VoiceJoined` when the joiner has one in that server
and hasn't had it played in the last 30 s. Each listener plays it through the soundboard's local playback at
Calls × **Join sounds** × the sound's volume, or falls back to the normal join cue (mixer row at 0 %, per-person mute,
old server, no sound).

**Tech Stack:** Rust (axum, sqlx/SQLite, ts-rs), Tauri 2 core, Svelte 5 + vitest.

**Spec:** `docs/superpowers/specs/2026-11-10-join-sounds-design.md`

## Global Constraints

- One join sound per person per server; none = the normal join cue. Nothing in profile settings.
- Set/clear only from the soundboard tile's right-click menu: **Set as my join sound** / **Clear my join sound**.
- Plays the full sound (≤ 6 s), through the soundboard path, for everyone in the call including the joiner.
- DM and group calls: normal cue. Leaving: normal leave cue, unchanged.
- Mixer: a **Join sounds** row (0 % = the normal join cue for everyone instead).
- Person menu on every surface: **Mute join sound** / **Unmute join sound**, per person, per device.
- Sound deleted → join sound cleared. At most once per 30 s per person (else the normal cue). Server muted/deafened
  joiners still get theirs. Ghost viewers produce no join, as today.
- 0.2.x clients keep the normal cue (they ignore the new field; `PROTOCOL_VERSION` stays 3).
- Existing tokens only; guard tests pass; public repo test people alex, sam, kim, jo.

## Builder's calls (with why)

- **Stored on `server_members.join_sound_id`** (`REFERENCES sounds(id) ON DELETE SET NULL`): "cleared on delete" and
  "cleared on leaving the server" fall out of the schema (Pulse has no leave-server route yet; deleting the row is what
  leaving will be). `ALTER TABLE … ADD COLUMN` is not a rebuild, so foreign keys being OFF at migration time changes
  nothing.
- **`Member.join_sound: Option<String>`** (the sound id), not a separate list: Ready already ships every server's
  members, `MemberUpdated` already reaches everyone, and every client then knows everyone's join sound (preloading and
  the menus need it). Old clients ignore the field. On `SoundDeleted` clients clear it locally (the server's `SET NULL`
  does the same), so no extra events.
- **Routes:** `PUT /servers/{id}/join-sound { sound_id }` (member; the sound must be in that server, else 400) and
  `DELETE /servers/{id}/join-sound`; both 204, both publish `MemberUpdated`; rate `sound_user`.
- **`VoiceJoined.join_sound: Option<JoinSoundPlay { sound_id, media_hash, volume, duration_ms }>`** (absent when
  none): the server decides (30 s rule), the event carries what to play so a listener needn't look it up.
- **30 s rule in memory** (`VoiceState::take_join_sound(user, now)`), per person across servers, marked only when a
  sound actually went out. A restart forgets it (harmless).
- **Your own join:** your cue normally plays when your connection comes up. If you have a join sound in that server
  and Join sounds is above 0 %, that cue waits for your own `VoiceJoined` (up to 3 s; it may also already have come)
  and plays its `join_sound`, or the normal cue if it has none or never comes.
- **Mixer:** `calls.join_sounds` (Calls section, after Soundboard, ▶ plays your join sound or else the first sound).
  The Soundboard notch doesn't apply to join sounds: each has its own row.
- **Per-person mute:** localStorage `pulse.joinSounds.mutes`, like the soundboard's. The item shows for someone else
  who has a join sound in a server you share, or whom you've muted (so you can undo it). In the voice menus it sits
  under Mute soundboard; in the member and DM menus beside Notify me when they go live.
- **Ring:** a join sound lights the joiner's ring while it plays, as a soundboard play does.
- **Preload:** joining a call already warms every sound of your servers; join sounds of that server's members go
  first.
- **If a join sound fails to play** (fetch error), the normal cue plays instead.

## Review Focus

- A sound deleted between being set and the join: the webhook reads the column at join time, `SET NULL` already ran →
  no join sound (server test `deleted_sound_clears_join_sound`).
- Joining a DM/group call with a join sound set in some server: no `join_sound` (server test).
- Rejoining within 30 s: second `VoiceJoined` has none (server test); a different person isn't blocked (unit test).
- Your own cue never plays twice and never goes missing: VoiceJoined before or after `connected`, or never (UI tests
  in `joinSounds.test.ts`).
- Setting a sound from another server (Favourites section shows sounds from several servers) sets it for the sound's
  own server (UI test on the menu passing `sound.server_id`).

---

### Task 1: Server — storage, routes, the join event

**Files:**
- Create: `server/migrations/0031_join_sounds.sql` (number = next free after rebasing)
- Modify: `protocol/src/rest.rs` (`Member`, new `SetJoinSoundRequest`), `protocol/src/gateway.rs` (`VoiceJoined`,
  new `JoinSoundPlay`), `protocol/src/lib.rs` (ts exports), `server/src/servers/routes.rs` (member queries),
  `server/src/sounds/routes.rs` (routes), `server/src/sounds/mod.rs` (`join_sound_for`), `server/src/voice/state.rs`
  (gate), `server/src/voice/routes.rs` + `server/src/voice/restore.rs` (event)
- Test: `server/tests/join_sounds.rs`, unit test in `server/src/voice/state.rs`

**Interfaces:**
- Produces: `Member { user, nickname, join_sound: Option<String> }`;
  `Event::VoiceJoined { channel_id, user_id, flags, join_sound: Option<JoinSoundPlay> }`;
  `JoinSoundPlay { sound_id: String, media_hash: String, volume: u16, duration_ms: u32 }`;
  `PUT/DELETE /servers/{id}/join-sound`, body `SetJoinSoundRequest { sound_id: String }`.

- [ ] Migration: `ALTER TABLE server_members ADD COLUMN join_sound_id TEXT REFERENCES sounds(id) ON DELETE SET NULL;`
  with a header comment saying exactly what it does and that FKs OFF changes nothing (no rebuild).
- [ ] Failing tests (`server/tests/join_sounds.rs`): `set_and_clear_reach_everyone` (PUT → 204, `MemberUpdated`
  with `join_sound == Some(id)`, Ready's members carry it; DELETE → `None`); `sound_from_another_server_is_refused`
  (400); `non_member_is_forbidden` (403); `unknown_sound_is_refused` (400); `deleted_sound_clears_join_sound`;
  `leaving_the_server_clears_it` (delete the member row, rejoin → `None`); `join_event_carries_the_sound` (webhook
  join → `VoiceJoined.join_sound` with the sound's hash/volume/length); `rejoin_within_30s_gets_the_normal_cue`;
  `dm_call_has_no_join_sound`; `muted_joiner_still_gets_it` (server mute then join).
- [ ] Unit test `join_sound_gate_once_per_30s_per_person` (`take_join_sound` true, false at +29 s, true at +30 s;
  another user unaffected).
- [ ] Implement; `cargo test -p pulse-server --test join_sounds` and the unit test pass; regenerate TS bindings.
- [ ] Commit `feat(server): join sounds: storage, routes, VoiceJoined carries the sound (#121)`.

### Task 2: Client core — set/clear commands

**Files:** `client/src-tauri/src/api.rs`, `client/src-tauri/src/commands.rs`, `client/src-tauri/src/lib.rs`,
test `client/src-tauri/tests/soundboard_e2e.rs`.

**Interfaces:** `Api::set_join_sound(&self, token, server: ServerId, sound: Option<&str>) -> Result<(), ApiError>`;
tauri command `set_join_sound(server_id: ServerId, sound_id: Option<String>)`.

- [ ] Extend `sounds_end_to_end` with set → clear against the real server (fails first: no method).
- [ ] Implement; the e2e test passes; xwin clippy clean. Commit `feat(core): set and clear your join sound (#121)`.

### Task 3: UI — state, playback, own-join hold, mixer row

**Files:** create `client/ui/src/lib/joinSounds.ts` (pure) + `joinSounds.test.ts`, `joinSounds.svelte.ts`; modify
`state.ts` (SoundDeleted clears members' `join_sound`), `mixer.ts` (`calls.join_sounds`), `voice.svelte.ts` (peer and
own cue routing), `soundboard.svelte.ts` (warm order), `tauri.ts` (`soundboardCmd.setJoinSound`),
`VolumeMixer.svelte`.

**Interfaces:**
- `joinSoundOf(s: AppState, serverId: string, userId: string): string | null`
- `peerJoinPlay(e, ctx: { myId, channelId, connection, level: number, mutes: Record<string, true> }):
  { kind: 'sound'; play: JoinSoundPlay } | { kind: 'cue' } | null` — null when it isn't someone else joining your
  call; `'cue'` when there's no sound, level 0, or muted.
- `holdOwnCue(s: AppState, channelId: string, myId: string, level: number): boolean`
- `loadJoinSoundMutes()`, `saveJoinSoundMutes()` (key `pulse.joinSounds.mutes`)
- `joinSounds.mutes`, `setJoinSoundMute(userId, on)`, `playJoinSound(play, userId)` (falls back to `playSound('join')`)

- [ ] Failing tests: peer sound / cue / null cases; mute and level 0 fall back to the cue; DM event (no join_sound) →
  cue; own join held only with a sound in that server and level > 0; own VoiceJoined before or after connected plays
  exactly one thing; timeout plays the cue; SoundDeleted clears; mixer default 100 and old records gain the notch;
  VolumeMixer shows the Join sounds row.
- [ ] Implement; `pnpm test` for these files passes. Commit `feat(ui): join sounds play for everyone in the call (#121)`.

### Task 4: UI — menus

**Files:** `Soundboard.svelte` (+ test), create `JoinSoundMuteItem.svelte`, modify `ParticipantMenu.svelte`,
`MemberMenu.svelte`, `ConversationMenu.svelte` (+ tests).

- [ ] Failing tests: tile menu shows **Set as my join sound**, or **Clear my join sound** on the current one, calling
  `set_join_sound` with the sound's own server; person menus show **Mute join sound** / **Unmute join sound** for
  someone with a join sound, never for yourself, and toggle the per-device store.
- [ ] Implement with the existing `.menu-item` markup; the tile menu's clamp accounts for the extra row.
- [ ] Full UI suite + `pnpm check`. Commit `feat(ui): join sound menu items (#121)`.

### Task 5: Verify, review, merge

- [ ] fmt, clippy -D warnings, server + client Rust tests, xwin clippy, full UI suite, `pnpm check`, types-check.
- [ ] Code review of the diff; fix what's real; deferrals get issues.
- [ ] Rebase on origin/main (migration number!), re-run, merge, push with `Closes #121`.
