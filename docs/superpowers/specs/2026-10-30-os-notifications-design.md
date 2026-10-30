# OS notifications (desktop toasts for messages)

Owner, 2026-10-30: "OS notifs can do (don't think I have to give much input there)." Everything below
not quoted from the owner is a default chosen for him; each is marked **(default)** so he can veto it.

## Why

When Pulse is in the background (a game, a browser), a DM or a mention only makes a sound. The sound
is easy to miss and says nothing about who or where. A system notification shows who wrote, where,
and (by default) what, and a click takes you straight there.

## Goals

- A Windows toast (Windows is ~99% of use) and a Linux desktop notification for the messages that
  already earn a message sound, while the Pulse window is not focused.
- Clicking it brings Pulse to the front and opens that conversation (and the quote's replies panel
  for a reply).
- Settings to turn notifications off, and to hide the message text.

## Non-goals

- Voice events (someone joined a call, went live), server events, reactions. **(default)**
- Notifications from a closed app: Pulse has no background service; if it isn't running, nothing.
- Read/reply actions inside the toast (quick reply). Maybe later.
- Mobile push.

## Anti-goals

- Spam: a busy group chat must not bury the desktop in toasts.
- Double alerts: no toast for what you're already looking at.
- Leaking: no message text on screen when the owner turned that off.

## Behaviour

**When.** A notification goes out for the messages `messageSound` (client/ui/src/lib/
conversations.ts) would ding for, ignoring the sound's global 2 s gap, with one extra rule: the window
is not focused (minimised, behind another window, or on another workspace). The "should this alert"
part of `messageSound` is split out into its own function. The sound adds its gap on top; the
notification adds focus and its per-conversation window. **(builder check: settled here)** So:

- DMs and groups: every normal message from someone else, unless the conversation is muted.
- Server channels: only messages that mention you (and replies to you, which count as mentions).
- Muted conversations or servers: mentions only.
- Never your own messages, system lines, or deleted ones.
- The sound still plays as today; the notification is in addition. **(default)**

**Not too many.** **(default)**
- The first message in a conversation notifies at once. That opens a 4 s window for the
  conversation. Messages arriving inside the window are counted. When the window ends, if any arrived,
  one notification replaces the first: the newest message, with "and N more" (for example "sam
  and 3 more"). That replacement opens a new window. **(builder check: settled here)**
- Focusing the window, or signing out, cancels any pending replacement and clears the windows.
- A new notification for a conversation replaces that conversation's previous one, where the OS
  supports replacing (Windows tag/group, Linux replaces-id). So the notification centre holds at most
  one entry per conversation.

**What it shows.** **(default)**
- Title: the author's display name. Add " in #channel (Server)" for a server channel, or " in Group
  name" for a group. A DM shows just the name.
- Body: a notification-only formatter. The conversation list's `previewText` stays as it is.
  **(builder check: settled here)**
  - Normally, the message text on one line, cut at 140 characters, with no author prefix (the
    title has it).
  - Custom emoji tokens show as `:name:` and mentions as `@name`.
  - A message with no text reads by what it carries: "sent a clip" (clip), "shared a quote" (quote;
    with text, the text), "posted the Quote of the Year card" (qoty_card).
- With "Show message text" off, the body is "New message" (or "Mentioned you").
- No avatar image in the first version (Windows needs a local file; can come later).

**Click.** Brings the window up (unminimise, show, focus) and opens that conversation. A reply opens
its quote's replies panel. Clicking after signing out, or for a conversation that no longer exists,
just brings the window up.

**Settings → Notifications** (new section). **(default)**
- "Desktop notifications": on by default.
- "Show message text": on by default.

Both are stored per device and per profile, like the other local preferences.

## How it gets made (builder's call where not stated)

- **Decision:** in the UI, next to the existing message-sound decision, as a pure function with
  tests. The UI already knows focus, mutes and the open conversation. A `notifyMessage(...)` returns
  `{title, body, channelId, threadId?}` or null, and the throttle lives beside it.
- **Delivery:** a Tauri command in the Rust core, `notify_message`, takes that payload and shows it.
  - **Windows:** `tauri-winrt-notification` (MIT/Apache).
    - App ID: the installed app's AppUserModelID; use PowerShell's in a dev run.
    - `on_activated` emits an event to the UI with the channel and thread.
    - Tag = channel id for replacement, group = "pulse".
  - **Linux:** `notify-rust` (MIT/Apache, D-Bus via zbus, no C libs).
    - A "default" action for the click.
    - The click is waited for on a worker thread, never the UI thread.
    - Replaces-id is kept per channel.
  - **Elsewhere:** no-op.
- **Click handling:** the core focuses the window (as the single-instance handler does) and emits
  `pulse://notification-click` with `{channelId, threadId}`. The UI navigates.
- **Failure:** an OS that refuses (notifications disabled for the app, no notification daemon) logs
  one warning per session and otherwise stays quiet. Messages still ding.
- **Licences:** add both crates to THIRD_PARTY.md if that file lists runtime crates this way; cargo deny
  must stay clean.

## Testing

- Unit tests:
  - the decision: every `messageSound` case, plus focus;
  - the title and body text, including privacy off, attachments, groups and servers;
  - the throttle with "and N more".
- A Rust test of the payload/command shape where practical.
- Manual, on Windows: a DM, a mention, a muted group (no toast) and a mention in it (toast), a burst
  of 10 messages (one toast with "and N more"), a click (window up, conversation open), privacy off,
  and notifications off.
- Manual, on Linux: the same, briefly.
