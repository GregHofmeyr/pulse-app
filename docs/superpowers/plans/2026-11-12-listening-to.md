# "Listening to…" and live Jam art Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.
> Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** An opt-in "Listening to <title> · <artist>" activity read from the local Spotify app's OS media session
(MPRIS on Linux, GSMTC on Windows), shown wherever status shows, and a live "Now playing" line on a Jam its sharer put
on the voice stage (#275).

**Architecture:** The client core runs an event-driven media-session watcher only while "Read media activity" is on,
settles its changes (a short delay before a pause clears, at most one send per few seconds), and sends a new gateway
frame `Listening`. The server keeps it in memory per connection (gone with the connection), fetches the album art
through the existing Spotify embed resolver when a track id is known, and announces it per viewer from the same pass
that announces presence, so whoever sees someone offline (Invisible, appear-offline-to, disconnected) never gets their
activity. A sharer's activity also updates their Jam on the stage (`StageNowPlaying`).

**Tech Stack:** Rust (axum, ts-rs protocol), Tauri 2 client core (zbus 5 on Linux, `windows` 0.62 WinRT on Windows),
Svelte 5 + vitest.

**Spec:** `docs/superpowers/specs/2026-11-12-listening-to-design.md`

## Decisions (builder's, within the owner's)

- **Two independent switches, Read is the master.** "Read media activity" off: no watcher, no D-Bus connection, no
  thread, nothing sent; "Show my activity" is greyed. Read on + Show off: Pulse still reads, but only the stage Jam card
  uses it (others never see it as activity). Read on + Show on: both. Both default off.
- **Visibility is presence's.** A viewer gets someone's activity only while that person looks online to them
  (`presence::view_for`), so Invisible, "appear offline to" and offline all show no activity. You see your own (if
  Show is on).
- **The stage Jam card is not "activity".** A Jam its sharer put on the stage shows what their Spotify plays whenever
  they read media, whatever Show or their status says: they're visibly in the call and shared the Jam themselves. Only
  Jam shares get it (not a shared song or album).
- **Per connection, latest wins.** Each connection holds its own; a person's activity is their most recently updated
  one (two devices both reading: the one that changed last). A connection closing takes its activity with it; a
  reconnect re-sends it (the "after Ready" frames).
- **Art only via the resolver.** With a Spotify id (`track:<id>` or `episode:<id>`, usually from MPRIS) the server
  unfurls `open.spotify.com/<kind>/<id>` (cached, own per-person art budget) and keeps its thumbnail; without one (GSMTC)
  there is no art and the link-out is "Open Spotify" to a search, not "Listen on Spotify".
- **Ads are nothing.** An ad (`/com/spotify/ad/…`) or an empty title clears the activity.
- **No migration**, nothing stored: activity is as ephemeral as the connection.
- **Old clients**: skip the unknown `ListeningChanged` / `StageNowPlaying` events and the new `Person.listening` /
  `StageShare.now_playing` fields; old servers ignore the unknown `Listening` frame.

## Constants (one place each)

- Client core `activity/mod.rs`: `SETTLE` 750 ms (a track change sends several signals), `CLEAR_AFTER` 8 s (pause or
  stop before it clears), `MIN_GAP` 5 s between sends (trailing send of the latest).
- Server `activity/mod.rs`: `MAX_TEXT` 200 chars per field; limits `listening_user` (10 at once, one per 2 s; a
  held-back one is sent when the bucket allows, like VoiceState) and `listening_art_user` (60 art fetches an hour).

## Tasks

### Task 1: Protocol

- [ ] `rest.rs`: `Listening { title, artist?, album?, spotify_id?, art?: EmbedThumb }`, `ListeningUpdate { title,
  artist?, album?, spotify_id? }`; `Person.listening` and `StageShare.now_playing` (`serde(default,
  skip_serializing_if)`, ts optional).
- [ ] `gateway.rs`: `ClientFrame::Listening { listening: Option<ListeningUpdate>, show: bool }` (Debug by hand),
  `Event::ListeningChanged { user_id, listening }` (per-viewer, never via `publish`), `Event::StageNowPlaying {
  channel_id, share_id, now_playing }` (channel-scoped). Round-trip tests; old-shape JSON still parses.
- [ ] `just gen-types`.

### Task 2: Server

- [ ] `activity/mod.rs`: `clean(ListeningUpdate) -> Option<Listening>` (trim, control chars out, caps, empty title or
  ad → None, `spotify_id` only `track|episode:<22 base62>`); tests.
- [ ] Hub: per-connection store (`set_listening` refuses an unregistered connection; `unregister` drops it), a
  per-connection sequence so a slow art fetch can't land over a newer update, `listening_of(user, shown_only)`.
- [ ] `sync_presence` gains the activity pass (per connection, only changes); `people_for` fills `Person.listening` and
  records it. Tests: invisible / hidden / offline viewers get none; turning Invisible sends `None`; back visible re-sends.
- [ ] Socket: the `Listening` frame, rate-limited with a trailing retry; the art fetch spawned; disconnect clears.
- [ ] Stage: `now_playing` on share creation (Jam only) and `listening_changed(user)` publishing `StageNowPlaying`.
- [ ] Integration tests (`server/tests`): two people over real sockets: activity in Ready and events, Invisible hides,
  rate limit holds back and then sends the latest, stage Jam gets now playing.

### Task 3: Client core

- [ ] `activity/mod.rs` (all platforms): `NowPlaying`, MPRIS metadata parsing (`mpris:trackid`, `xesam:url`, title,
  artists, album), GSMTC filtering (`is_spotify(app_id)`, status codes), the settle state machine (pure, tested with a
  paused clock), `MediaActivity` state: `set(read, show)` creates/drops the watcher, emits `pulse://media-activity` for
  Settings, sends the frame, and supplies the after-Ready frame.
- [ ] `activity/mpris.rs` (Linux): zbus `PropertiesChanged` on `org.mpris.MediaPlayer2.spotify` +
  `NameOwnerChanged` for that name; test against a fake player on the session bus (skipped without one).
- [ ] `activity/gsmtc.rs` (Windows): manager `SessionsChanged`, the Spotify session's `MediaPropertiesChanged` /
  `PlaybackInfoChanged`, a worker thread woken by those events only; compiles under xwin clippy.
- [ ] `set_media_activity` command; `declare` closure adds the frame.
- [ ] Measure CPU/RSS with the watcher on vs off (fake player, idle and a burst of track changes); report.

### Task 4: UI

- [ ] `lib/activity.ts`: listen link + label, line text; state: `Person.listening` from Ready, `ListeningChanged`,
  offline presence clears, `StageNowPlaying` updates the share. Tests.
- [ ] `lib/activityPrefs.svelte.ts` (localStorage, both off) and Settings → Privacy "Activity" with two switches, the
  one-line explanation and what Pulse currently reads.
- [ ] Activity line in the profile card (art, Spotify icon, link-out), member list and group members rows, DM header.
- [ ] Stage strip / corner card / pill: "Now playing" line and art.
- [ ] Full UI suite + `pnpm check`; guard tests.

### Task 5: Finish

- [ ] fmt, clippy, server + client tests, xwin clippy, UI; code review of the diff; fix; rebase; merge `Closes #275`.
