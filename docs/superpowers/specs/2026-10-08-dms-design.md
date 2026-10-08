# DMs, group DMs, unread & mentions — design

Date: 2026-10-08 · Status: approved in conversation, pending written-spec review
Sub-project 1 of 4 in the "social" bundle (1 DMs → 2 ringing calls → 3 keybinds → 4 categories).

## 1. Goal

Friends message each other privately, 1:1 and in small groups, outside any server — and Pulse tells you,
quietly and privately, what you haven't read yet, everywhere (DMs, groups and server channels).

**Success:** a friend DMs you, you see it's unread and hear one sound, you reply; from that DM you "Add people" and a
group chat works end to end — renamed, someone added later, someone removed, someone leaving — with unread counts and
@mentions behaving everywhere.

**Settled (don't re-litigate):** DM/group schema and member-only privacy via `audience_for()` (original spec §4–5);
groups are max 10 people; a 1:1 DM with someone you already have one with reopens it (existing `POST /dms`);
**no read receipts** — nobody can see whether you read anything; Pulse is invite-only, so anyone with an account can
be DMed.

## 2. Decisions from the brainstorm

| Topic | Decision |
|---|---|
| Group creation | Discord-style: **"Add people" in a 1:1 DM creates a new group** (you + them + picked people); the DM stays. "Add people" inside a group adds to it. |
| Group permissions | **Flat**: any member can add, remove anyone, rename; anyone can leave. No owner. Every change posts a system line. |
| Unread | **Everywhere**: DMs, groups and server channels. Private to you, synced across your devices. |
| Notifications | **Sound only**, no desktop pop-ups: DMs, groups, and @mentions in server channels. Server channels otherwise silent (unread dot). |
| Who can be DMed | Anyone with an account; start from a member list ("Message"), the Home board, or "New message". |
| Unread architecture | Server stores last-read; initial sync carries counts; client counts live from events; mark-read call. |
| Home | **"Your people" board** (mockups 2026-10-08): conversation list on the left; when nothing is open, the main area is a board of everyone. |

## 3. Data (migration `0002`)

Existing tables put to use: `channels.name` (group name), `channel_members`, `read_states`, `mentions`,
`notification_prefs`, `messages.kind = 'system'` with `system_payload`.

New / changed:

- `notification_prefs`: add `muted_until TEXT NULL` (RFC 3339; NULL with `muted = 1` = forever). Timed mutes: 1 h,
  8 h, forever.
- `dm_hidden(user_id, channel_id, hidden_at)` PK `(user_id, channel_id)`: "Close conversation". A row is deleted when a
  new message arrives in that channel or when the user re-opens/starts it.
- `users.last_seen_at TEXT NULL`: set when a user's **last** gateway connection closes.

## 4. Server API

### 4.1 Directory & sync

- Initial sync (`Ready`) gains:
  - `users: Vec<User>` — everyone (id, username, avatar), plus `last_seen_at` and online presence;
  - `read_states: Vec<ReadState { channel_id, last_read_message_id, unread, mentions }>` for every channel you can see;
  - `mutes: Vec<Mute { target_kind, target_id, until }>`;
  - `hidden: Vec<ChannelId>` (closed conversations).
- Events: `UserCreated { user }` (registration), `UserSeen { user_id, last_seen_at }` (went offline).

### 4.2 Groups

- `POST /dms { user_ids }` (exists): 1 other → reuse/create DM; ≥ 2 others → **always a new group**. "Add people" from a
  DM calls this with the DM partner + picked people.
- `POST /channels/{id}/members { user_ids }` — add to a group (members only; `group` kind only; total ≤ 10).
- `DELETE /channels/{id}/members/{user_id}` — remove anyone (members only); removing yourself = leave. The last member
  leaving deletes the channel.
- `PATCH /channels/{id} { name }` — rename a group (members only; 1–64 chars, trimmed; empty = back to default name).
- System messages (`kind = system`, `system_payload` JSON): `{type:"group_created", by, with:[…]}`,
  `{type:"members_added", by, users:[…]}`, `{type:"member_removed", by, user}`, `{type:"member_left", user}`,
  `{type:"group_renamed", by, name}`.
- Events: `GroupMembersChanged { channel_id, user_ids }` (to remaining + added members), `ChannelUpdated { channel }`
  (rename), `ChannelRemoved { channel_id }` (**only** to the removed/leaving user; after it, they receive nothing for that
  channel). Added users receive `ChannelCreated` and can load the **full history**.

### 4.3 Unread, mentions, mutes, close

- `POST /channels/{id}/read { message_id }` — sets your last-read (monotonic: never moves backwards). Emits
  `ReadStateUpdated { channel_id, last_read_message_id }` **to your own sessions only** (multi-device sync).
- Mentions: on send, `@username` tokens are matched (case-insensitive) against users **who can see the channel**
  (server channel: everyone; DM/group: members); matches go into `mentions`, and `MessageCreated` carries
  `mentions: Vec<UserId>`. A mention of someone who can't see the channel is plain text — no ping, no record.
- `PUT /mutes { target_kind, target_id, until: null | RFC3339 }` and `DELETE /mutes/{kind}/{id}`. Events
  `MuteUpdated` to own sessions only.
- `POST /channels/{id}/close` (DM/group only) → `dm_hidden`; `ConversationHidden { channel_id }` to own sessions. A new
  message in that channel deletes the row server-side (clients un-hide on `MessageCreated`).
- Counts in `Ready` are computed with SQL: messages after `last_read_message_id` not authored by you, excluding
  system messages; mentions = rows in `mentions` for you after that point. Read points start at "all read":
  the migration sets every existing user's read point in every channel they can see to its latest message, and
  joining a server / being added to a group / a new channel being created sets the newcomer's read point to that
  channel's latest message at that moment (no 500-message unread wall). A missing row is treated the same way.

### 4.4 Privacy (the `audience_for` rules this adds)

All new events go through `audience_for()`; the own-sessions-only events (`ReadStateUpdated`, `MuteUpdated`,
`ConversationHidden`) target exactly the acting user. Removed members stop receiving channel events immediately and
lose read access (404) to its messages.

## 5. Client

### 5.1 Home — "Your people"

- Left panel: "Direct messages" header, **New message** button, conversation list sorted by latest activity:
  avatar (DM) or two-avatar stack (group), name, last-message preview, red unread count; muted rows dimmed.
- Main area with nothing open: **"Your people"** board — everyone as cards, online first, then by last seen; each card
  shows status ("online", a voice icon + "in <server> · <voice channel>" from server voice state only, "last seen 2h ago"), unread
  highlight, and actions **Message** / **Call** (Call disabled until sub-project 2) / **Join** (when they're in a server
  voice channel you can see).

### 5.2 Conversation

- Header: DM → avatar + name; group → face-pile + **inline-editable name**; buttons **Add people** (person-plus icon,
  no emoji), **mute** (bell), **⋯**.
- Groups get a right-hand **members panel**; hover → ✕ to remove (with the system line as the audit trail).
- Message list: system lines styled small/grey; a red **NEW** divider at your last-read point; messages that mention
  you are tinted; `@name` rendered as a pill; composer `@` autocomplete limited to people who can see the channel.
- ⋯ menu: Mute (1 h · 8 h · forever / Unmute), Rename group (groups), Close conversation, Leave group (groups, danger).
- **Picker** (Add people / New message): search, chips for picked people, checkboxes, people already present disabled,
  "N of 10 people" counter, primary button "Create group" / "Add" / "Message".

### 5.3 Unread everywhere

- Top tabs: Home shows a red count (unread DMs/groups + mentions); a server tab shows a white dot for unread channels
  and a red count for mentions.
- Channel list: unread channels bold; mention count pill.
- "Read" = conversation open **and** window focused **and** scrolled to the bottom (or the "N new" jump clicked).
- Mute: no count/sound; @mentions still count and ping.

### 5.4 Sounds

New `message` UI sound (native, like the existing ones). Plays for DMs, groups and @mentions — unless that
conversation is open with the window focused, or it's muted — never for your own messages or system lines; bursts are
rate-limited to one sound per 2 s.

## 6. Edge cases

- Removed while viewing → conversation closes with "You were removed from <group>"; history inaccessible.
- Last member leaves → group deleted.
- Picker would exceed 10 → further choices disabled; server returns 400 too.
- Mentions of non-members → plain text.
- Reconnect → counts recomputed from `Ready` (no drift).
- Renames of a DM (1:1) are rejected (DMs show the other person's name).

## 7. Testing

- **Server (integration):** privacy first — non-members can't read/post/receive; removal cuts events and history
  immediately; added members see history; mentions never leak. Groups: add-from-DM creates a new group and leaves the
  DM intact; add/remove/leave/rename + system lines; 10-cap; last-leave deletes. Unread: `Ready` counts and mentions;
  mark-read is monotonic and syncs to own sessions only; mutes (timed expiry) and mentions-through-mute; close/reopen
  on new message. Last seen: set only when the last connection closes.
- **Client (vitest):** pure functions for unread/mention counting, read rule (focus + bottom), sound decision +
  rate limit, conversation sort, picker limit, `@` parsing/rendering.
- Existing suites green: `just check`, `just voice-it`, Windows cross-check.
- **Ear/eye test (Greg):** two profiles — DM, Add people → group, rename, add, remove, leave, mute, close, mention;
  watch counts and sounds.

## 8. Out of scope

Ringing calls (sub-project 2; the board's Call button stays disabled until then), keybinds (3), categories (4),
desktop notifications, read receipts (never), group icons.
