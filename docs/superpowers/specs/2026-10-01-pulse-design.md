# Pulse — design spec

**Date:** 2026-10-01 · **Status:** draft for review
**Display name:** Pulse · **Technical name:** `pulse-app` (repo, crates, binary, process — avoids clashing with PulseAudio)
**UI mockups:** https://claude.ai/artifact/VkSTZTs5L3KzaKmxwShAki (dark-mode v1 views)

## 1. Purpose

A lightweight, self-hosted Discord alternative for one private friend group (~10 people). It replaces Discord for
**voice** and **day-to-day text**; Discord stays around for media (clips, images, files).

**Success looks like:**
- The group uses Pulse for evening voice calls instead of Discord.
- The client uses **under 150 MB RAM in a 4-person call** (Discord often uses 2 GB+).
- It runs on the **Windows** machines the friends use and on Greg's **Linux/Hyprland** dev machine.
- Hosting costs nothing or close to it (target: Oracle Cloud Always Free, Johannesburg region).
- Building it is fun, and a way for Greg to learn Rust.

**Non-goals (v1):** public sign-up, media/file uploads, a homemade voice engine or codec, mobile clients, macOS builds,
real permissions, end-to-end encryption.

## 2. Users & trust model

- Invite-only, one friend group. Everyone trusts each other: **no admin role, anyone can do anything** (create
  servers/channels/roles, rename anyone, assign roles).
- DMs and groups are **private from other users**, but **not from the server operator**. Whoever hosts it can read the
  SQLite file, and LiveKit decrypts voice to forward it. Users must be told this plainly. E2EE is parked (§13).

## 3. Architecture

```
┌──────────────── Client (Tauri 2) ─────────────────┐
│  Svelte 5 UI (webview)  — dumb renderer           │
│     ▲ Tauri events            │ Tauri commands    │
│  Rust core                                        │
│   ├─ session token (OS keychain)                  │
│   ├─ gateway client (WebSocket) + REST client     │
│   ├─ VoiceManager: LiveKit Rust SDK (libwebrtc)   │
│   ├─ hotkeys (Windows) / local IPC socket (Linux) │
│   └─ outbox (offline message queue)               │
└───────┬──────────────────────────────┬────────────┘
        │ HTTPS + WSS                  │ WebRTC (UDP, Opus)
┌───────▼──────────────┐  webhooks ┌───▼────────────┐
│ pulse-app server     │◄──────────│ LiveKit server │
│ Rust: axum + SQLite  │  tokens ─►│ (voice SFU)    │
└──────────────────────┘           └────────────────┘
```

### 3.1 Repo layout (Cargo workspace, monorepo)

| Crate / dir | Purpose |
|---|---|
| `server/` | axum HTTP + WebSocket server, SQLite via sqlx, LiveKit token minting + webhook receiver, badge engine |
| `client/` | Tauri 2 app: `src-tauri/` (Rust core), `ui/` (Svelte 5 + Vite) |
| `protocol/` | Shared types: REST request/response bodies, gateway events, IDs. TypeScript types for the UI are **generated** from it (ts-rs or specta); CI fails if the generated file is stale |

### 3.2 Technology choices

| Concern | Choice | Why |
|---|---|---|
| Server language | Rust (tokio, axum) | Lightweight, and a learning goal |
| Database | **SQLite** (WAL mode, `STRICT` tables), sqlx with compile-checked queries | One binary + one file; trivial backups; in-memory test DBs. LiveKit does not use our DB (it is in-memory single-node; Redis only for multi-node) |
| Passwords | argon2id | Standard |
| Voice | **LiveKit** server (self-hosted, single Go binary) + LiveKit **Rust** SDK in the client | Battle-tested SFU, Opus, NAT traversal, TURN, and screen share for v2 |
| Client shell | **Tauri 2** | Uses the OS webview (WebView2 on Windows, WebKitGTK on Linux), not a bundled Chromium |
| UI | **Svelte 5** | Small and fast; easy to pick up coming from React |
| Image processing | `image` crate → WebP | Re-encoding removes metadata and neutralises malicious files |

### 3.3 Key principles

- **The UI is a dumb renderer.** The token, connections and audio live in the Rust core. A webview bug (or XSS) cannot
  steal the session token or drop the call.
- **REST for actions, WebSocket for pushes.** Write to the DB first, *then* publish the event.
- **Voice never touches our server.** Audio flows client ↔ LiveKit directly.
- **No polling anywhere.** The only periodic traffic is the gateway heartbeat.

## 4. Data model (SQLite, `STRICT`)

IDs are **ULIDs** (text, time-sortable). History pagination is "50 older than ID X".

**Accounts**
- `users(id, username UNIQUE, password_hash, avatar_hash NULL, status [online|dnd], created_at)`
- `sessions(token_hash PK, user_id, created_at, last_used_at, expires_at)`: 90-day expiry that renews on use; only
  the SHA-256 hash of the token is stored
- `invites(code PK, created_by, created_at, used_by NULL, used_at NULL)`: single-use; anyone can create one

**Servers & channels**
- `servers(id, name, icon_hash NULL, created_by, created_at)`: all servers are visible to every user; joining is one click
- `server_members(server_id, user_id, nickname NULL, joined_at)`
- `channels(id, server_id NULL, kind [text|voice|dm|group], name NULL, position, created_at)`: `server_id` is NULL for DMs/groups
- `channel_members(channel_id, user_id, added_by, added_at)`: **only** for `dm`/`group`; defines who can access them
- `nickname_history(id, server_id, target_user_id, changed_by, old_nick NULL, new_nick NULL, changed_at)`: **never deleted**; this is the Name Archive

**Roles (cosmetic)**
- `roles(id, server_id, name, color, position, hoist BOOL, permissions INTEGER DEFAULT -1)`: `permissions` is
  reserved and unused; `-1` = everything
- `member_roles(server_id, user_id, role_id)`
- Display colour = colour of the member's highest-positioned role. Hoisted roles get their own group in the member list.

**Messages**
- `messages(id, channel_id, author_id NULL, kind [normal|system], content, reply_to_id NULL, system_payload NULL, created_at, edited_at NULL, deleted_at NULL)`
  - **System messages** cover renames, missed calls and badge unlocks; `system_payload` is JSON.
  - **Soft delete:** content is wiped and `deleted_at` set; replies show "original message deleted".
- `reactions(message_id, user_id, emoji, created_at)`, PK on all three. Unicode emoji only in v1.
- `mentions(message_id, user_id)`: parsed from `@username` when the message is sent
- `read_states(user_id, channel_id, last_read_message_id)`: unread and mention counts are derived from this

**Notifications**
- `notification_prefs(user_id, target_kind [server|channel], target_id, muted BOOL)`

**Media**
- `images(hash PK, bytes BLOB, created_at)`: avatars and server icons, 128 px WebP, served at `/img/<hash>.webp` with
  `Cache-Control: public, max-age=31536000, immutable`

**Activity & badges**
- `voice_sessions(id, user_id, channel_id, joined_at, left_at NULL)`: **server voice channels only**
- `user_badges(user_id, badge_key, earned_at, first_earner BOOL)`
- Badge *definitions* live in code (§9), not the DB.

**Ephemeral (in memory only):** who is in voice, mute/deafen state, typing, presence, ringing calls.

## 5. Real-time gateway

- **Connect:** `GET /gateway` (WebSocket, permessage-deflate). The client sends `Hello{token}`. The server replies
  `Ready{me, servers, channels, members, roles, dms, read_states, voice_states (visible only), badges}`.
- **Reconnect:** the client fetches a fresh `Ready` and re-loads the latest page of the open channel. There is no
  event replay; at this scale the snapshot is a few KB.
- **Fan-out:** every outbound event goes through **one function, `audience_for(event) -> Set<UserId>`**:
  - server-scoped events → every user (all servers are public)
  - `dm`/`group` events → that channel's `channel_members` only
  - this function is the single place privacy is enforced, and the most heavily tested code in the project
- **Client → server over the WebSocket:** `Typing{channel}` (throttled to 1 per 3 s; receivers fade it after 6 s),
  `VoiceState{muted, deafened}`, `CallAccept/CallDecline{channel}`, `Heartbeat`.
- **Presence:** heartbeat every 30 s; offline after 60 s with none. Several connections per user are allowed.
- **Optimistic sends:** the client sends a `nonce` with each message; the server echoes it so the pending message is
  matched up.

## 6. Voice

### 6.1 Rooms & tokens
- Each voice channel, DM and group is a LiveKit room named after the channel ID.
- `POST /voice/{channel}/token` → a short-lived LiveKit JWT, **only** for users allowed to access the channel.
- LiveKit **webhooks** (`participant_joined/left`) → `POST /livekit/webhook`. The signature is **verified**. The
  server updates its in-memory voice state, writes `voice_sessions` (server channels only), and broadcasts through
  `audience_for`.
- **One voice connection per client.** Joining another channel or accepting a call leaves the current one.

### 6.2 Server channels vs DM/group calls
- **Server voice channels:** silent. No ring, no chat line, no ping. Visible to anyone viewing the server.
- **DMs and groups behave the same:** starting a call **rings every other member** (in-app card, ringtone, OS
  notification if the window is minimised). It gives up after 30 s. Members who didn't join get a "Missed call from X"
  system message.
- **No busy signal:** a caller cannot tell whether the callee is already in another call.
- "In voice" indicators elsewhere in the UI show **server voice channels only**.

### 6.3 Audio pipeline
- Opus ~32 kbps mono, **DTX** on (silence sends almost nothing), **in-band FEC**, LiveKit **RED** for packet-loss recovery.
- libwebrtc audio processing: **AEC3** echo cancellation, noise suppression, AGC2, high-pass filter, transient
  suppression. Each is a user toggle.
- Built by us: **input gain** (0.5×–4×), **input sensitivity gate** with a level meter, **per-user output volume**
  (0–200%), "mute for me", and a **mic test / echo bot**.
- **Open question for the spike:** whether we own mic capture (capture → gain → APM → gate → LiveKit
  `NativeAudioSource`, with the speaker output fed to AEC as reference) or let libwebrtc own the devices. The spike
  decides; per-user volume has to work either way.

### 6.4 Devices
- Input and output pickers in Settings, plus a quick switcher in the call bar. A "System default" option follows the OS.
- Switching devices mid-call doesn't drop you.
- If a device is unplugged: fall back to the default and show a notice. When it reappears, switch back to it.
  Selections are remembered per machine by device name.

### 6.5 Mute / deafen / hotkeys
- **Mute** stops sending. **Deafen** silences all incoming audio *and* mutes the mic; undeafen restores the previous
  mute state.
- **Global hotkeys:** toggle mute and toggle deafen. Windows: Tauri global-shortcut plugin. Linux/Hyprland: the app
  listens on a local Unix socket; `pulse-app --toggle-mute` / `--toggle-deafen` sends to it and is bound in Hyprland
  config. The in-app UI does **not** advertise the Linux method.
- No push-to-talk in v1.

### 6.6 Network resilience & latency
- Bandwidth: a 6-person call is usually **under 100 kbps down**.
- Voice reconnect is handled by the LiveKit SDK: ICE restart first (also covers Wi-Fi↔Ethernet), then a full rejoin.
  UI states: 🟡 unstable → 🟠 reconnecting → 🔴 disconnected + retry.
- Per-participant connection quality bars, from LiveKit quality events.
- Gateway reconnect: exponential backoff 1 s → 30 s with jitter; reconnect **immediately** on an OS network-up event
  or window focus.
- **Outbox:** messages sent while offline queue in the Rust core and show as *pending*, then *sent*, or *failed →
  retry*. Never lost silently.
- Restrictive networks fall back to LiveKit's built-in TURN over TCP/TLS.
- Latency is mostly geography: host in Johannesburg; overseas friends add ~150–180 ms, which is acceptable.

### 6.7 Sounds
- Join, leave, mute, unmute, deafen, undeafen, ringtone and badge unlock sounds are **bundled with the client**
  (CC0 or homemade, a few KB each) and played by the UI. No server storage.

## 7. Text features (v1)

| Feature | Notes |
|---|---|
| Send / read, paginated history | 50 per page |
| Edit / delete own messages | soft delete |
| Replies | quote above; "original deleted" handled |
| Typing indicator | ephemeral |
| Unread indicators | per channel, per server tab, per DM |
| @mentions | autocomplete on `@`; highlight; mention badge counts. **No OS notifications for messages in v1** |
| Reactions | Unicode emoji, small picker component |
| Markdown | **sanitised** (strict allow-list; no raw HTML). Links clickable; **no link previews** |

**Mute & DND:** mute applies per server, DM or group. Muted means no sounds, no notifications and no ringing, but the
unread dot and mention badge still show. **Do Not Disturb** (user status) silences everything app-wide; incoming calls
still appear in the app, silently.

## 8. Identity: nicknames, roles, avatars, profiles

- **Nicknames** are per server, and anyone can change anyone's. Every change → a `nickname_history` row plus a system
  message in the server's first text channel ("Sam renamed Alex → Big Dog").
- **Name Archive:** a server-wide view and a per-person timeline on the full profile.
- **Roles:** cosmetic; anyone can create or assign; colour + hoist.
- **Avatars:** upload → crop → 128 px WebP → content-addressed URL (§4). Changing your avatar broadcasts the new hash.
- **Two profile views:**
  - **Quick card** (in-server popover): banner, avatar, status, nickname + username, "In <voice channel>", roles,
    "View full profile", quick-message box.
  - **Full profile:** everything above plus the Name Archive, stats (voice hours, times renamed, most-used reaction,
    member since) and a badge trophy case. Stats count **server activity only**.

## 9. Badges (achievements)

- **The server awards every badge.** The client cannot unlock one.
- Engine: badge definitions in code (`key, name, icon, hidden, rule`). Each rule is a pure function over
  `(event, user stats, clock)`. Rules run after each relevant event or webhook.
- On unlock: `user_badges` row → a system message in the server's first text channel + an unlock sound. The first
  earner gets `first_earner = true` (gold border, permanently).
- **Hidden badges** show as "???" until earned. **Rarity** shown as "2/8 have this".
- **Privacy:** never earned from DM/group activity; never reveals it.
- **v1 ships the engine plus ~5 badges**, chosen from: Night Owl (in voice at 3 am), Marathon (6 h straight in voice),
  Identity Crisis (renamed 10×), Renamer-in-Chief (renamed others 25×), Ghost (joined and left voice in under 5 s,
  10×), Necromancer (replied to a message over 30 days old), Reaction Farmer (100 reactions received), a hidden
  logo-click egg.

## 10. Client UI

Matches the canvas mockups (§ header link).
- **Layout:** pill tabs across the top (**Home** first, then one tab per server, then "+"); rounded panels underneath:
  channel/DM list | content | member list. Custom title bar with window controls.
- **Theme:** dark only in v1. Layered greys `#141519 → #34363d`, **no true black**, rounded frames (10–16 px), Onest
  font, an **accent colour picker** in settings. Light theme / full theming is parked.
- **Views:** server text channel, Home (DMs + groups), voice/call view with tiles, incoming call card, Settings (Voice
  & Audio, Keybinds, Notifications, Profile, Account, Appearance), quick card, full profile, Name Archive.
- Svelte stores mirror Rust-core state, which is pushed via Tauri events. The UI calls Tauri commands, never the
  network directly.

## 11. Security checklist

- argon2id passwords; session tokens are 256-bit random values, only their hashes stored, kept in the OS keychain on
  the client.
- Every REST handler and WebSocket subscription checks access; non-members get **404** for private channels.
- Markdown sanitised with an allow-list; Tauri CSP locked down; the UI has no direct network access.
- LiveKit webhook signatures verified; LiveKit tokens are short-lived and room-scoped.
- Uploaded images are always re-encoded; size limit enforced before decoding.
- Rate limits on login, invite redemption and message sends (generous; a guard against bugs and scripts, not strangers).

## 12. Testing & CI

**TDD throughout. Privacy tests are the highest priority.**

1. **Privacy:** table tests for `audience_for` (event type × viewer type). Integration tests (in-process server,
   in-memory SQLite, real HTTP and WebSocket clients):
   - an outsider gets 404 on a DM's messages
   - an outsider is refused a call token
   - an outsider's WebSocket receives **zero** private events, proven with a trailing public marker event
   - badges and voice hours are unaffected by DM activity
   - forged webhooks are rejected
2. **Server units:** badge rules with an injected clock; an XSS payload corpus against the sanitiser; nickname
   history; unread and mention counts; soft delete; session expiry and renewal.
3. **Client core:** `VoiceManager` against a fake LiveKit room behind a trait (mute/deafen state machine, device
   fallback, single-connection rule); outbox and backoff with a fake clock.
4. **UI:** Playwright against the Svelte UI with mocked Tauri IPC; component tests for mention autocomplete, the
   emoji picker and the call card.
5. **Manual voice/network checklist:** `lk` CLI fake participants, the echo bot, two local instances on different
   devices, `tc netem` (200 ms latency, 10% loss, 20 s link down) with expected UI states, and a Windows smoke test
   on Greg's Windows partition before each release.
6. **Footprint budget:** measure RSS idle and in a 4-person call on Linux and Windows for each release. **Target:
   under 150 MB in a call.** Investigate any regression before shipping.

**CI (GitHub Actions):**
- `just check` runs the same checks locally (optional pre-push hook).
- Workflows run on PRs and on `main` only, with cancel-in-progress concurrency, path filters (server/client/protocol)
  and `Swatinem/rust-cache`.
- Jobs: fmt, clippy, `cargo test`, Svelte type checks, Playwright, a check that generated TS types are up to date.
- The **Windows `.exe` is built only on release tags or manual dispatch.**
- No `pull_request_target` workflows.

**Repo visibility:** start **private**; go **public** before Windows release builds start, for unlimited free Actions minutes.
Go-public checklist:
- [ ] Branch protection on `main`
- [ ] Require approval for fork PR workflows
- [ ] Secret scanning + push protection on
- [ ] Confirm no secrets/config in history (`.env`, keys, server addresses)
- [ ] No licence file (all rights reserved) unless decided otherwise
- [ ] Issues on/off decided

## 13. Delivery order (high level; the implementation plan details it)

0. **Voice spike (throwaway):** the LiveKit Rust SDK on Linux **and** Windows. Join a room, publish the mic, play
   remote audio, select devices, per-user volume, and decide the capture-pipeline question (§6.3). Testers: two local
   instances + `lk` fake participants.
1. Workspace skeleton, protocol crate + TS generation, CI, `just check`.
2. Server: auth, invites, sessions, servers/channels/members, messages, gateway with `audience_for`.
3. Client: Tauri shell, login, tabs/layout, text channels.
4. Voice: LiveKit deployment (local docker), tokens, webhooks, `VoiceManager`, devices, mute/deafen, hotkeys, sounds.
5. DMs and groups + ringing calls.
6. Identity: nicknames + archive, roles, avatars, quick card + full profile.
7. Remaining text features: replies, reactions, mentions, unread, typing, markdown.
8. Badges engine + first ~5 badges; stats.
9. Settings polish, mute/DND, footprint measurement, Windows release build, hosting on Oracle Free (JHB).

## 14. Parked (explicitly out of v1)

Screen share (v2) · push-to-talk · OS notifications for messages · link previews · **custom emoji** · Discord OAuth
login · real permissions · private servers · **E2EE** (voice via LiveKit E2EE, then text) · custom per-user join
sounds · RNNoise / stronger AI noise suppression · light theme / full theming · idle presence · message search (SQLite
FTS5) · more badges and easter eggs · a Zig CLI bot (the coworker challenge) · mobile clients · multi-region hosting.

## 15. Open questions

- Capture pipeline ownership (decided by the spike, §6.3).
- Whether LiveKit's Rust SDK exposes per-participant output volume directly, or we mix audio ourselves (spike).
- The exact first ~5 badges (Greg + friends to pick).
- Oracle Free signup availability in the JHB region. Fallback: LiveKit Cloud free tier for voice + any free host for
  the API.
