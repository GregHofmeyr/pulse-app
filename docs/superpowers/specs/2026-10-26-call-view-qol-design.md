# Call view and quality-of-life batch: design

Date: 2026-10-26. Status: for the owner's review (builder check done, answers folded in).

Comes after: screenshare first cut (merged), reactions + emoji (merged). Comes before: watching several streams at
once (its own later push, §2.5), quotes.

## 1. Intent

The voice channel's main area is where the group spends most of its time. Today it is a grid of user tiles, and
someone streaming only gets a small LIVE badge by their name; getting into a stream takes a detour. Discord and Root
both make streams first-class in that grid, give instant feedback when you join, and show a connection status you can
click for network details. The owner wants Pulse to feel at least as responsive, plus a few small comforts noticed in
Discord and Root.

### Goals

- From the call grid, one click watches a stream, a second click makes it full view.
- Every click in the voice flow gives immediate visible feedback; no "click, nothing, then it happens".
- Connection health is visible at a glance and inspectable in one click.
- The layout and state are ready for watching several streams at once, without building that now.
- Failed actions are visible on the thing that failed and recoverable.

### Non-goals (this push)

- Watching more than one stream at a time (§2.5: prepared for, capped at one).
- Camera video (the grid toggle in §2.4 counts streams only until cameras exist).
- Profiles (the right-click menu has no "View profile" until the profiles push).
- Message attachments (they don't exist yet; §6 sets the rule they must follow).

### Anti-goals

- **Heavier calls.** Nothing here may add steady CPU or bandwidth while sitting in a call. The stats panel only polls
  while it is open; grid previews use the low-quality layer.
- **Regressing known Root bugs** (from their 0.9.151 changelog): animations running off-screen or unfocused for hours
  (window-drag stutter), CPU/memory growing over long sessions, the open channel flashing its unread dot while you're at
  the bottom of it. §7 lists the checks.

## 2. The call view

### 2.1 Grid

- One **user tile** per person in the channel, as today (avatar, speaking ring, mute/deafen badges, connection bars,
  volume label).
- One **stream tile** per live stream, placed right after its streamer's user tile:
  - not watching: the streamer's avatar on a dimmed backdrop, a red **LIVE** tag, a **Watch** button, and a viewer
    count (eye icon + number, hidden at 0);
  - watching: the tile becomes a live **preview** of the stream (low-quality layer), with a small hover toolbar:
    stream volume, quality (**Auto / Data saver**, as today's viewer), **Stop watching**, and **Full view**. The preview
    always uses the low layer; the quality setting only decides what full view asks for. **My call.**
- **Your own stream** gets a stream tile too, showing your local preview (today's LiveCard), with its existing "hide my
  preview" toggle. It never counts as a watched stream.
- Tiles keep a 16:9 shape and the grid reflows (1, 2, 3 columns by width), like today's grid.

### 2.2 Full view

- Clicking a watching preview (or its Full view button) makes that stream take the main area, at the quality the viewer
  picked (Auto asks for the high layer).
- A slim strip of small user tiles runs along the bottom so you can see who's talking (speaking rings work there too).
  **My call.**
- **Back** (button top-left) or **Esc** returns to the grid; the stream keeps playing as a preview.
- Today's stream viewer becomes this full view (same controls and diagnostics overlay). There is no pop-out yet; that
  is a later push.

### 2.2a Leaving the call view while watching (owner's choice)

- Opening a text channel, DM or any other page while watching keeps the stream going: its audio keeps playing and a
  small floating **mini-preview** sits in a corner of the main area (draggable between corners, remembered).
- Clicking the mini-preview goes back to the call view (to full view if that's where you were). Its ✕ stops watching.
- So the watched stream's video element belongs to the shell, not the grid: it moves between the grid tile, full view
  and the mini-preview without restarting.
- **Watch from elsewhere (my call):** "Watch" in a stream toast or the channel list's LIVE popover opens the call view
  straight in full view on that stream.
- **A stream in a voice channel you're not in (my call):** its tile shows **Join & watch**, which joins that voice
  channel (leaving your current one, as joining always does) and then opens the stream.

### 2.3 Right-click menus (user tiles, stream tiles, and people in the channel list's voice section)

- **On someone else:** a volume slider (0–200 %, as today's tile popover), **Mute for me** (local only, as Discord's),
  **Message** (opens the DM); if they are live: stream volume, Watch / Stop watching, Full view.
- **On yourself:** Mute, Deafen, Go Live / Stop streaming, Voice settings.
- Keyboard: the menu opens with the context-menu key or Shift+F10 on a focused tile; arrows move, Enter picks, Esc
  closes.
- "Mute for me" persists per person like volumes do. **My call.**
- The menu is one component reused by the grid and the channel list.

### 2.4 "Show non-streaming participants" toggle

- A toggle in the grid's header (eye icon). Off: only stream tiles (and, later, camera tiles) show; user tiles hide.
  On (default): everyone.
- Remembered per user. When it is off and nobody is live, the grid shows a short "Nobody's streaming — show everyone"
  line with a button.

### 2.5 Ready for several streams (prepared, not built)

- Watch state becomes a list (`watching: Watching[]`), each entry with its own streamer, element, volume, quality and
  layer, instead of today's single `watching`.
- A constant `MAX_WATCHED = 1` caps it. With the cap reached, Watch on another stream stops the oldest watch first
  (today's behaviour: switching).
- Grid previews ask for the low layer and full view for the requested quality, per entry, so raising the cap later is
  the main change. The multi-watch push owns decoding/bandwidth budgets and its own optimisation run.

## 3. Instant feedback when joining voice

- The click shows you in the channel immediately: in the channel list's voice section and in the grid, your entry is
  greyed (60 % opacity) with a small spinner, labelled "Joining…".
- When the connection completes, the grey and spinner go; on failure, your entry is removed and the status (§4) shows
  the error with a **Retry** button.
- Switching channels shows you leaving the old one and joining the new one the same way (the old entry disappears at
  once).
- The client core reports join stages to the UI, in order:
  1. **Requesting access** (asking the Pulse server for a voice token),
  2. **Connecting to voice** (LiveKit signalling + WebRTC/ICE — normally the longest),
  3. **Starting audio** (devices open, first frames out),
  4. **Connected**.
  Plus **Reconnecting** (with the attempt number) and **Disconnected**. The stage names above are the UI copy.
- Only you see the greyed "Joining…" entry. Other people see you appear normally when the server learns you've joined
  (the existing `VoiceJoined`), with no protocol change. **My call.**
- **Microphone trouble is not a failed join (my call):** "Connected" means the voice connection is up. "Starting audio"
  shows until the first microphone frames go out or the microphone reports an error; a microphone error leaves you
  connected (you can still listen) with the existing microphone error shown, not removed with Retry.

## 4. Connection status you can click

- The existing bottom-left voice block ("Voice connected · channel") shows the current stage from §3 while joining and
  turns into a button once connected.
- Clicking opens a small panel:
  - **Ping** (round-trip time to the voice server), **jitter**, **packet loss** (%), updated every second while the
    panel is open, each coloured green/amber/red (**my call:** ping 0–99/100–199/200+ ms; jitter 0–19/20–49/50+ ms;
    loss 0–1/1–5/5+ %);
  - a 60-second ping sparkline;
  - **Voice server** (the domain) and **connection type** (UDP / TCP / TURN relay);
  - **Copy details** (a plain-text block with the above plus app version, for bug reports).
- **What's measured (my call):** all three describe *your* path to the voice server, so they work even when you're
  alone in the call:
  - ping = the WebRTC round-trip time on your connection to LiveKit;
  - jitter and loss = what LiveKit reports back about the audio you send (remote-inbound stats); loss % = lost ÷
    (lost + received) over the last few seconds.
  The existing per-peer numbers in `client/src-tauri/src/voice/netstats.rs` (incoming jitter, concealed audio) stay in
  the logs and diagnostics.
- **Cost (my call):** the core keeps the last 60 s of ping as a tiny ring buffer fed by the stats it already samples, so
  the sparkline is full when the panel opens; the 1-second refresh only runs while the panel is open.
- "Voice server" means the LiveKit host (e.g. `voice.<domain>`).
- Text channels with no voice connection show no block, as today.

## 5. Small comforts

### 5.1 Shift-click to pick several emoji

- In the picker opened from the composer: Shift+click (or Shift+Enter) inserts the emoji and keeps the picker open with
  the search box focused; a plain click inserts and closes, as today.
- In the reaction picker: Shift+click toggles that reaction and keeps the picker open.
- Frequently used counts each pick.

### 5.2 Animated emoji freeze while you stream

- While you are live, animated emoji show their still frame everywhere, like the rest of Pulse's animations already do
  (`.streaming` in app.css). They resume when you stop.

## 6. Failed actions

- **Unsent messages** render like a real message (your avatar, name, markdown, emoji), faded to 60 %:
  - sending: a small "Sending…" under it;
  - failed: "Failed to send" in red with **Retry** and **Discard**.
- **Unsent messages survive a restart:** they are saved with the composer drafts and come back failed on next launch
  (never resent automatically). **My call.**
- **Failed edits** keep the edit box open with the error under it and your text intact; **failed deletes** and
  **failed reactions** show a small inline error on the message (auto-hides after 5 s) instead of the channel-wide
  banner. The banner stays for errors that aren't about one message.
- **Rule for future media** (image quotes, clips, attachments): an upload in progress shows its preview faded with a
  progress bar and Cancel; a failed one shows "Failed to upload" with Retry and Discard; nothing is lost on a dropped
  connection. Those pushes build it; this push only writes the rule down here.

## 7. Tests

- UI units: grid ordering (stream tile after its streamer), not-watching → watching → full view → back, own stream
  tile, viewer count, toggle on/off and the empty-state line, right-click menu contents for self/other/live, keyboard
  opening, `MAX_WATCHED` switching, join stages rendering (greyed entry, failure removes it with Retry), status panel
  colours and thresholds, shift-click keeps the picker open (composer and reactions), emoji frozen while streaming,
  unsent message render/Retry/Discard/survives reload, inline errors for edit/delete/react.
- Core: join stages emitted in order for a normal join, a failed token request, a failed ICE connect, and a reconnect;
  stats panel polling stops when closed.
- Long-session checks (Root's bugs): no animation or polling runs while unfocused; memory stays flat over a 1-hour idle
  call with a stream preview open (manual, with the existing diagnostics).
- Manual (Windows): join feel on a real network, watching from the grid, full view and back, right-click menus, the
  stats panel numbers against Settings → Advanced diagnostics.

## 8. Left to the builder

- Exact tile sizes, spacing, animations, icons.
- Where the stats come from inside the core if `netstats.rs` lacks a value (connection type may need the ICE candidate
  pair).
- How unsent messages are stored with drafts.
