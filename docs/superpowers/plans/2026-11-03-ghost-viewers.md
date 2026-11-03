# Ghost viewers: implementation plan (#46)

**Goal:** a server member who isn't in a voice channel can watch a stream there as a ghost, when the stream allows it.
Nobody else learns who: the streamer and everyone else see only a 👻 count.

**Spec:** `docs/superpowers/specs/2026-10-15-screenshare-design.md` §4.4 (state, `ViewersChanged`), §5.1–5.4 (rules,
tickets, anonymity, changes mid-stream). The first cut (`2026-10-23-screenshare-first-cut.md:31`) left all of this out.
Pop-out (`#v2`, #61) isn't built, so there is no ghost `#v2` ticket.

## Decisions

- **Ghost state is in server memory only** (`streams::state`): opaque id `g_<16 random bytes, base32>` → user,
  channel, session, grant ids, the streams it watches and its `#v` connection. Never in SQLite: ghost tickets don't go
  through `livekit_grants` (that table stores the identity next to the session). Their `pulse_grant` is checked
  against the in-memory record on the join webhook instead.
- **One ghost session per (login session, channel)**, from the first ghost ticket until its `#v` leaves the stream
  room or the server removes it. A ticket refresh inside it reuses the id; the next watch after it gets a new one.
  Sessions never joined are dropped after the ticket TTL (10 min).
- **The room-wide rule.** A LiveKit ticket can subscribe to every track in `<channel>~s`, so per-stream permission
  can't be enforced by LiveKit. Ghosts are admitted only while every live stream in the channel allows ghosts (and
  at least one is live). A stream going live, or switching, with ghosts off removes every ghost from the room. This is
  stricter than the spec's per-stream rule, never looser; per-stream (publisher track permissions) is a follow-up issue.
- **Explicit ghost requests.** `StreamTicketRequest.ghost` and `WatchRequest.ghost` (default false). A voice member
  asking as a ghost gets 403, and a non-member asking as themselves still gets 403 (so the ticket-before-webhook retry
  never turns into a ghost ticket by accident).
- **Ghost tickets:** room `<ch>~s`, identity `g_<id>#v` / `#a`, `can_publish: false`, no sources,
  `can_publish_data: false`, `can_subscribe: true`, 10 min. Never a voice-room ticket. Same rate limit as named.
- **Counts:** `StreamInfo.ghosts` and `ViewersChanged.ghosts` (u32). `StreamSettings.allow_ghosts` (serde default
  true).
- **Log:** `ghost watch` (user, channel, ghost id, streamer) at info on every ghost watch. Nothing else names a ghost.
- **Removal (LiveKit `RemoveParticipant` on `g_<id>#v` and `#a`):** allow-ghosts off; the ghost joins voice in that
  channel (they reconnect as `<uid>#v`/`#a`); the channel is deleted; the login session is revoked (logout,
  sign-out-others, password change); `streams::ghosts::member_removed` for leaving a server (no leave/kick route
  exists yet: it's tested directly and wired when one lands, see issue).
- **Previews:** a non-voice member may fetch one only if that stream allows ghosts (spec §4.2).

## Client

- Rust: `stream_ticket(…, ghost)`, `stream_watch(…, ghost)`, `stream_audio_watch(…, ghost)`; the `#a` connection
  remembers whether it's a ghost and reconnects when that changes.
- UI store: a watch is a ghost watch when you aren't in that voice channel and the stream allows ghosts. The `#v`
  room remembers it's a ghost room. Allow-ghosts switched off (or a voice-only stream starting in that channel) ends
  ghost watches there with "<name> switched to voice-only" (also when LiveKit's removal lands first). Joining voice in
  that channel re-watches the same streams as yourself.
- Go Live dialog: "Allow ghost viewers" (on by default, remembered). The live card: the 👻 count and the same toggle
  mid-stream (an update through go-live). Popover, tile: 👻 count; Watch as a ghost when allowed, else Join & watch.
- Ghost stream audio: with no call there is no voice mixer, so a ghost's `#a` plays through a small output of its own.

## Tests (TDD)

- Server state unit tests: id format and freshness, reuse within a session, never derived from the user id, counts,
  purge rules, never-joined expiry.
- Server integration (`tests/stream_ghosts.rs`, fake LiveKit): ticket identity/permissions; voice members can't get
  ghost tickets and ghosts never get voice tokens; ghosts refused when a stream disallows them; no gateway event or
  Ready carries the ghost's user id or a `g_` id; toggle-off removes ghost participants and zeroes the count; joining
  voice removes the ghost; logout removes it; join webhook admission and refusal; member removal; rate limit.
- UI (vitest): ghost watch path (ghost ticket/watch/audio flags), the 👻 count on the card/popover/tile, the
  voice-only notice (event first and LiveKit first), the voice-join upgrade, the dialog toggle.

## As built

- Counts follow LiveKit: a ghost counts once its `#v` join webhook has been admitted (a ticket and a watch with no
  connection are nobody), and stops when that `#v` leaves.
- A ghost watching elsewhere keeps watching when its voice channel changes; joining the ghost's own channel upgrades.
  The upgrade waits (up to 15 s) for the call to connect, and also runs when LiveKit's removal arrives first.
- A ghost's stream audio plays through `stream::ghost_out` (the chosen output device) when there's no call, with the
  volume mixer's Calls × Streams level.

## Deferred (issues)

- #148: allow ghosts per stream (publisher track permissions) instead of per stream room.
- #149: wire `streams::ghosts::member_removed` into a leave-server / remove-member route (none exists yet).
- #150: ghost watches survive a server restart (and the CLI password reset can't reach ghost state).
- #61 (comment): the ghost `#v2` ticket comes with pop-out.
