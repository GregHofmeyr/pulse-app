# Spotify on the Stage Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Anyone in a server voice call can share a Spotify Jam, song, album, playlist, episode, show or artist to the
call's stage; everyone sees it as a strip above the tiles (or a pill / corner card), joins with one click, and it goes
when the sharer stops it, leaves the call, or after 3 h (#234).

**Architecture:** The server keeps one share per voice channel in memory (beside the voice state, which is in memory
too), validates the link through the existing embed unfurler, and announces it with two new gateway events plus a field
in `Ready`; a share also posts a system line in the channel's chat. The client keeps the shares in `AppState`, and a
per-device view state (strip / pill / corner / hidden) decides how the stage shows it.

**Tech Stack:** Rust (axum, sqlx/SQLite, ts-rs protocol), Tauri 2 client core, Svelte 5 + vitest.

**Spec:** `docs/superpowers/specs/2026-11-09-spotify-stage-design.md`

## Global Constraints

- Owner's (pick) items live in one place each so they can change fast: server `server/src/stage/mod.rs` constants
  (`MAX_PER_CHANNEL = 1`, `SHARE_TTL = 3 h`, `ENDS_WHEN_SHARER_LEAVES = true`, `ANYONE_IN_CALL_MAY_STOP = true`) and the
  client's `client/ui/src/lib/stageShare.ts` mirror (`ANYONE_IN_CALL_MAY_STOP`, `JAM_MAY_HAVE_ENDED_MS = 1 h`).
- Spotify rules (#196): no Spotify API keys, no remote control or listen-along; the single-colour `SpotifyIcon`
  (≥ 21 px, never over art); approved wording only: "Join the Jam", "Listen on Spotify", "Open Spotify"; artwork
  uncropped (`object-fit: contain`, 4 px corners) with nothing over it. No coloured left bar on any Spotify card
  (owner, 2026-11-10): remove it from the #196 chat card too.
- Link validation only through `embeds::Unfurler::unfurl`; only Spotify links (open.spotify.com, `spotify:socialsession:`,
  `spotify.link` short links that resolve to Spotify) are accepted, checked before anything is fetched.
- Old (0.2.x) clients: they skip unknown events and ignore unknown `Ready`/`Message` fields; the share line's `content`
  reads as plain text: `🎵 alex shared a Jam to the stage`.
- No migration (in-memory shares: a share is as ephemeral as the call it belongs to; a restart drops it and the sharer
  re-shares; persisting would need reconciling with the voice state LiveKit restores after a restart).
- UI: existing tokens and components only; icon sizes 14/16/18; motion via tokens with reduced motion respected; guard
  tests (motionCss, colorTokens, Icon sizes) pass. Test people alex, sam, kim, jo; made-up Spotify ids in tests.

## Review Focus

- A Jam link pasted as `spotify:socialsession:<id>` or with `?si=` tracking: accepted, stored as its normalised web
  link (test in Task 2).
- Someone shares while a stream is in full view: the card stays hidden for them and comes back as it was when they
  leave full view (test in Task 6).
- The sharer moves to another voice channel (LiveKit sends a leave for the old room): the old channel's card goes
  (test in Task 3).
- A Stop sharing click racing a new share: the DELETE names the share id, so it never clears the newer one (Task 2).
- A shared thumbnail outliving the share: its media row's cleanup clock is restarted on share (24 h grace > 3 h)
  (Task 2).

---

### Task 1: Protocol types and events

**Files:**
- Modify: `protocol/src/rest.rs`, `protocol/src/gateway.rs`, generated `client/ui/src/lib/protocol/*.ts`
- Modify: every `Message { … }` literal (`server/src/messages/{routes,system}.rs`, `server/src/clips/routes.rs`,
  `server/src/polls/routes.rs`, server tests), the `Ready` literal in `client/src-tauri/src/gateway.rs` tests

**Interfaces:**
- Produces (rest.rs):
  - `StageShare { id: String, channel_id: ChannelId, shared_by: UserId, shared_at: String, expires_at: String, embed: LinkEmbed }`
  - `StageShareRequest { url: String }`
  - `StageShareLine { by: UserId, kind: String }` (the Spotify kind: `jam`, `track`, …)
  - `Message.stage_share_line: Option<StageShareLine>` (serde default, skip if none, ts optional)
- Produces (gateway.rs): `Event::StageShared { share: StageShare }`, `Event::StageShareCleared { channel_id, share_id: String }`
  (both channel-scoped in `channel_id()`), `Ready.stage_shares: Vec<StageShare>` (serde default, ts optional).

- [ ] Add the types; run `cargo test -p pulse-protocol` (ts-rs export) and check the new `.ts` files appear.
- [ ] Fix all literals; `cargo check --workspace --tests`.
- [ ] Commit `Spotify stage: protocol (#234)`.

### Task 2: Server share state, routes, line

**Files:**
- Create: `server/src/stage/mod.rs` (constants, `StageState`, `spotify_link`, `phrase`), `server/src/stage/routes.rs`
- Modify: `server/src/lib.rs` (module + router), `server/src/voice/state.rs` (`pub stage: StageState` in `VoiceState`),
  `server/src/config.rs` (`stage_share_ttl: Duration`, default `stage::SHARE_TTL`; tests set short), `server/src/limits.rs`
  (`stage_share_user: Rate::new(6, 0.1)`), `server/src/messages/system.rs` (`STAGE_SHARE_LINE` payload helpers),
  `server/src/messages/routes.rs` (read the line back), `server/src/gateway/socket.rs` (`stage_shares` in Ready)
- Test: `server/tests/stage.rs`

**Interfaces:**
- `PUT /channels/{id}/stage-share` body `StageShareRequest` → `200 StageShare`. 400 not a server voice channel or not a
  Spotify link / no card; 403 not in the server or not in the call; 429 over the limit. Posts the line and publishes
  `StageShared`; replaces (and announces `StageShareCleared` for) the share it pushes out. The same person re-sharing
  the same link that's up: returns it, no line, no event.
- `DELETE /channels/{id}/stage-share/{share_id}` → 204; 404 when that share isn't up; 403 not in the call (or not the
  sharer when `ANYONE_IN_CALL_MAY_STOP` is false). Publishes `StageShareCleared`; no line.
- `stage::left_call(s, channel, user)`, `stage::channel_gone(s, channel)` for Task 3.
- `fn spotify_link(raw: &str) -> Option<String>`: the normalised link when it's a Spotify one (`links::extract` on the
  trimmed input, then `providers::detect`: `Known` Spotify, or `Short` on host `spotify.link`).
- `fn phrase(kind: &str) -> &'static str`: jam "a Jam", track "a song", album "an album", playlist "a playlist",
  episode "an episode", show "a podcast", artist "an artist", else "something".

- [ ] Write `server/tests/stage.rs` (fake provider + `spawn_with`): share a track from open.spotify.com (card fields,
  line content `🎵 alex shared a song to the stage`, payload, event to sam); a Jam via `spotify:socialsession:` and via
  the fake `spotify.link`; replace (cleared event for the old id, one share left); same link again is a no-op;
  non-Spotify link 400 without a fetch; not in the call 403; jo (not in server) 403; DELETE by sam (in the call) works,
  stale id 404, out of the call 403; Ready lists the share; the expiry (short `stage_share_ttl`) clears it with an event;
  the thumb's media row `created_at` is refreshed.
- [ ] Run: `cargo test -p pulse-server --test stage` → fails (no routes).
- [ ] Implement; run → passes. Commit `Spotify stage: share, replace, stop and expiry on the server (#234)`.

### Task 3: Leaving the call and deleting the channel clear it

**Files:** Modify `server/src/voice/routes.rs` (leave branch with `in_memory`), `server/src/servers/channels.rs`
(voice channel delete). Test: `server/tests/stage.rs`.

- [ ] Tests: the sharer's leave webhook clears it (event); a leave from someone else doesn't; a leave for another
  channel (moved) clears only the old channel's; deleting the channel drops it (Ready no longer lists it).
- [ ] Implement via `stage::left_call` / `stage::channel_gone`; pass; commit.

### Task 4: Client core commands

**Files:** `client/src-tauri/src/api.rs` (`share_to_stage(token, channel, url) -> StageShare`,
`stop_stage_share(token, channel, share_id)`), `commands.rs` (`share_to_stage`, `stop_stage_share`), `lib.rs`
(register), `client/ui/src/lib/tauri.ts` (`api.shareToStage(channelId, url)`, `api.stopStageShare(channelId, shareId)`).
Tests: api.rs unit test against a mock like its neighbours if the file has that pattern.

- [ ] Implement, `cargo test -p pulse-client` + xwin clippy; commit.

### Task 5: Client state and pure helpers

**Files:**
- Modify: `client/ui/src/lib/state.ts` (`stageShares: Record<string, StageShare>`: Ready, `StageShared`,
  `StageShareCleared` by id, dropped with `ChannelDeleted`/channel removal)
- Create: `client/ui/src/lib/stageShare.ts` (constants; `stageCard(share, sharerName, nowMs) -> { title, subtitle, action, jam, kindLabel }`;
  `mayStop(share, meId, inCall)`; `shareLinePhrase(kind)`), `client/ui/src/lib/stageShareView.svelte.ts` (per-device
  `{ mode: 'strip'|'pill'|'corner'|'hidden', shareId }` in localStorage `pulse.stageShare`; `modeFor(shareId)`: a new
  id shows `strip` unless the saved mode is `hidden`; `setMode`)
- Test: `state.test.ts`, `stageShare.test.ts`, `stageShareView.svelte.test.ts`

Copy: Jam title "Spotify Jam", subtitle "Listen together · Shared by alex", after 1 h "Shared by alex · may have
ended". Others: title = embed title (else the link), subtitle "<author or kind> · Shared by alex". Action from
`spotifyCard` (#196).

- [ ] Tests first, then code; `pnpm vitest run` on them; commit.

### Task 6: Stage surfaces

**Files:**
- Create: `client/ui/src/components/StageSpotify.svelte` (strip / pill with hover preview / corner card; Stop sharing,
  minimise, back to strip, ✕), `client/ui/src/components/StageShareBox.svelte` (popover: paste a link, Share, inline error)
- Modify: `VoicePanel.svelte` (header Spotify button, green while shared and hidden; strip above the grid; pill/corner
  in the stage's bottom-right; nothing while `fullStreamer`), `LinkEmbed.svelte` ("Share to stage" on Spotify cards
  in a voice channel's chat while you're in its call; no left bar for Spotify), `MessageItem.svelte` (the share line
  from its payload with live names)
- Test: `StageSpotify.test.ts`, `StageShareBox.test.ts`, `VoicePanel` render test, `LinkEmbed.test.ts`,
  `MessageItem` line test

- [ ] Tests: strip shows title/sub/button/Shared by; minimise → pill; hover pill previews corner; click opens corner;
  ✕ hides, header button turns green and brings it back; a new share shows the strip again unless hidden; Stop
  sharing calls the API; hidden while a stream is in full view and back after; reduced motion; LinkEmbed button only
  in the call; paste box errors inline.
- [ ] Implement; full UI suite + `pnpm check`; commit.

### Task 7: Finish

- [ ] fmt, clippy (server, client, xwin), all tests; code review; fixes; issues for anything deferred; merge.
