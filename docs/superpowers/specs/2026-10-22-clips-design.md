# Clips (phase 1: trim and share): design

Date: 2026-10-22. Status: for the owner's review (builder check done, answers folded in; replay format pending, §3.1).

Depends on: the **media store** (`2026-10-22-reactions-emoji-design.md` §5), whose R2 location this push builds;
**reactions**; the **replies side panel** from the quotes design (§3.6 there), reused. **Built after both the
reactions and emoji push and the quotes push have merged**; the plan is written against their real code. Independent
of screen share.

## 1. Intent

The group shares game clips constantly, in bursts when a new game is fresh. Today:

1. NVIDIA Instant Replay saves a 2-minute clip into `Videos\<Game>\`.
2. They drag it into **Clipchamp** to trim it. The owner loves and hates Clipchamp in equal measure: great free editing,
   but minutes to launch, Microsoft sign-in gates, slow imports and very slow exports.
3. They download the trimmed file and drag it into Discord.

Discord also struggles to play clips older than about a year, and the group still watches clips that are 3–4 years
old.

Phase 1 cuts out the middleman: **drop the replay into Pulse, trim it right there, send.** Clips are kept forever and
always play.

Phase 2 (actually capturing clips in Pulse) is a maybe. It needs research on whether anyone would use it, and it is
**not** part of this design.

### Goals

- From dropping the file to sending takes seconds plus the upload, never minutes.
- Trimming is precise and can cut out a boring middle.
- Clips are easy to browse later, by game and by person.
- A 4-year-old clip plays exactly like today's.

### Non-goals

- Capturing clips (phase 2).
- Effects, text overlays, music, transitions, speed changes: this isn't Clipchamp.
- Clips in normal text channels (attachments in general are a separate future feature).
- **Sending clips from Linux** (agreed with the owner): the editor and sending are **Windows only** in phase 1, where
  the group games and records. Linux plays clips; its "Share a clip" says it's Windows-only for now.

### Anti-goals

- Anything slow to start or slow to export.
- Expiring links or clips that silently stop playing.
- Surprise storage costs.

## 2. Decisions made with the owner

| Topic | Decision |
|---|---|
| Flow | Drag the replay onto the clips channel (or pick a file) → the editor opens at once on the local file → cut → caption and game → send. |
| Trimming | **Cut tool:** "Cut here" splits at the playhead; drop pieces; the kept pieces join up. Playback always skips dropped pieces. No Start/End/Loop buttons. |
| Editor controls | Play, ±5 s, frame step, Cut here, Drop piece, Undo; icons, not emoji. |
| Where it's processed | **On the sender's PC** with ffmpeg; the original never uploads. |
| Game | **Read from the clip's folder** (NVIDIA saves into a folder named after the game); a picker with the games the group has used, and "Add game" grows that list. |
| Caption | Optional. |
| Quality | Presets with size estimates; **default Light (720p60)**; Sharp (1080p60) and Original available. |
| Length | **Up to 1:30** after trimming. |
| Keeping | **Forever.** |
| Storage | **R2** (the media store's R2 location, built in this push). Budget: the owner has a $3 R2 alert; expected use is about 200 clips a year, a few GB. |
| Storage visibility | Total clip storage shown in server settings. |
| Channel | **Clips only.** Chat about a clip goes in the **replies side panel** (as in quotes). |
| Views | **A toggle between a list** (like a text channel, newest at the bottom) **and a video wall** (3 across, newest first). Remembered per person per channel. New clips channels open in the **wall**. |
| Wall previews | **No hover previews.** Cards show a still thumbnail; click to play. |
| Loading | Paged as you scroll (both views); only cards on screen are drawn. |
| Filters | Chips at the top: All, per game, and By person. |

Approved mockups (git-ignored, owner's machine): `.superpowers/brainstorm/284608-1792674052/content/clips-v2.html`
(the cut-tool editor and both channel views).

## 3. The editor

### 3.1 Opening

- Dropping a video file on a clips channel (or "Share a clip" in its header → file picker) opens the editor as a large
  modal. Supported inputs: MP4/MOV/MKV/WebM with H.264/HEVC/AV1/VP9 video (what NVIDIA, Xbox Game Bar, OBS and phones
  produce). Anything ffmpeg can't read gets a clear error.
- **Preview preparation (my call, after the builder check):** WebView2's player can't play every input (MKV, and
  HEVC/AV1 depend on installed extensions). Files it can play open at once. Others get a short **"Preparing
  preview…"** step: a quick repackage when only the container is the problem (seconds), or a fast low-resolution
  preview encode on the GPU when the codec is. The final export always works from the original file.
- **Pending:** the owner checks tonight whether NVIDIA Instant Replay files are H.264 or HEVC (and whether the file's
  Title holds the game). If H.264 MP4, preparation is an edge case; if HEVC, it's the common path and the manual test
  must cover it.
- The header shows the file name, length, resolution, fps and size.
- **Game guess:** the parent folder's name (for example `Videos\Overwatch 2\…` → "Overwatch 2"). "Desktop" and
  unknown names aren't guessed. A guess that matches a known game (ignoring case) selects it; a new name is offered as
  "Add 'X'".

### 3.2 Cutting

- A filmstrip timeline with time ticks and a playhead; the preview plays the local file.
- **Cut here** splits the piece under the playhead. Click a piece to select it; **Drop piece** drops it (shown hatched
  red); clicking a dropped piece brings it back. Piece edges can be dragged to fine-tune. **Undo** steps back through
  cuts and drops.
- Play skips dropped pieces; the readout shows the kept time ("0:12 / 0:41 kept") and the piece count.
- Keyboard: Space play/pause, ←/→ 5 s, `,`/`.` one frame, C cut, Delete drop, Ctrl+Z undo.
- **Over 1:30 kept:** the readout turns red and Send is disabled with "Clips can be up to 1:30. Drop a bit more."

### 3.3 Sending

- Side panel: caption, game, quality (each preset shows an estimated size for the kept length), **Trim & send**.
- Quality presets (**my calls** on exact settings): **Light** 720p, keep up to 60 fps, about 4 Mbps; **Sharp** 1080p,
  up to 60 fps, about 8 Mbps; **Original** keeps the source resolution and fps at a high bitrate. Never upscale.
  Audio: AAC 160 kbps stereo. Output: MP4 (H.264 for playback everywhere, with `faststart` so it starts playing before
  it's fully downloaded).
- **Encoding (my calls):** ffmpeg bundled with the app (no install). It uses the GPU encoder when there is one (NVENC
  on the group's NVIDIA cards; others where available) and falls back to the CPU. Cuts are frame-accurate (the clip
  is re-encoded, which also joins the pieces cleanly).
- After Trim & send, the editor closes. The clip appears in the channel at once as a **pending card** with progress:
  "Preparing… 45%" then "Uploading… 60%". Cancel is available until it's posted. Chatting carries on meanwhile.
- If the app closes or the network drops mid-upload, the card shows "Upload failed — Retry" and keeps the processed
  file until retried or cancelled (**my call:** resumable upload is nice to have, not required).
- Also made locally and uploaded with the clip: a **thumbnail** (a frame from about 1 s into the kept clip) and the
  duration, width and height.

## 4. The channel

### 4.1 Common

- A new channel kind, **`clips`** (server channels), created like any other.
- The main view holds clips only. Each clip has: poster, time, caption, game, duration, reactions (normal reactions,
  any number per person), and "View N replies" which opens the **replies side panel** (shared with quotes).
- Header: filter chips (All, each game used in the channel, "By person ▾"), the view toggle (list/wall icons), and
  "Share a clip".
- The poster can edit the caption and game afterwards, and delete the clip (which deletes its file and replies).
- Paging: about 30 clips per page, more as you scroll; only on-screen items are drawn.

### 4.2 List view

- Like a text channel: newest at the bottom, each clip a message (poster avatar and name, game tag, time, caption,
  the player inline at a comfortable width, reactions, replies link).
- New clips since you last looked: the usual "NEW" line.

### 4.3 Wall view

- 3 across (2 on narrow windows), newest at the top left, scrolling down.
- Each card: the thumbnail with duration and a fullscreen button, then caption, poster, game tag, reactions.
- Clicking the thumbnail plays the clip in the card with sound; fullscreen is available. One clip plays at a time.
- Unseen clips get a small "new" badge.

### 4.4 Toggling

- The toggle is remembered per person per channel; new clips channels open in the wall.
- Switching keeps the clip you were looking at in view.

### 4.5 Playback (Discord-style)

- Play/pause, seek bar with time, volume (remembered across clips), fullscreen. Space and arrow keys when focused.
- Streams progressively from the server; seeking works before the whole file is downloaded.
- **Always playable:** links are never stored in messages. The client asks for a fresh, short-lived link each time
  it plays a clip, so a clip from years ago plays like a new one.

## 5. Storage

- Clips (and their thumbnails) go to the media store's **R2 location**, built in this push: the S3-compatible client,
  credentials in the server's `.env`, and **login-only access** (the server hands out short-lived signed URLs to logged-in
  members; R2 URLs are never public).
- **A separate bucket and token (my call, after the builder check):** clips get their own R2 bucket with a token scoped
  to it, apart from the backups bucket (whose prune job deletes files by age). New `.env` names, added to
  `deploy/env.example`. The owner creates the bucket and token in Cloudflare once; the plan includes the steps.
- **Not content-addressed (my call):** the server never sees the bytes, so clips get a server-assigned id as their R2
  key instead of a hash. The media row records the kind, location, size and that it's a clip. Clips are always streamed
  through a fresh signed link and never go into the client's own media cache (only the normal short-lived HTTP cache).
- Thumbnails can live on R2 too or on the server disk (builder's choice; they're small).
- **Uploads:** straight from the client to R2 with a signed upload URL the server issues after checking the request
  (member of the server, size within limits), so the clip doesn't pass through the small VM. The upload runs in the
  Tauri core (Rust), not the webview, so the bucket needs no CORS rule and progress is reported to the UI. The server confirms the
  object exists and its size before the clip is posted.
- **Limits (my calls):** 1:30 duration (checked on the client and against the uploaded metadata), 250 MB per clip
  (Original at 1:30 fits), rate limit 20 clips per hour per person.
- **Server settings → Storage:** that server's clip storage (GB, count) and its share of the estimated monthly R2 cost.
  Visible to everyone in the server (Pulse's rules are flat).
- **The all-servers total** (storage and estimated cost) is for the owner only: a `just` command reading the database,
  not a page, because Pulse has no instance-owner role and one server's settings mustn't show other servers' usage.
- Backups: R2 is the durable copy for clips; the nightly backup doesn't copy them (the database rows that point at
  them are backed up). **My call.**

## 6. Data and events (for the builder)

- `channels.kind` gains `'clips'`.
- A clip is a message in a clips channel with: media references (video and thumbnail), duration, width, height,
  caption (the message content), game id.
- `games(id, server_id, name, created_by, created_at)`, unique `(server_id, lower(name))`: the registry the picker and
  filters use.
- A per-user, per-channel view preference (list or wall).
- Pending uploads: a clip row in a "pending" state visible only to its poster until confirmed, or kept client-side
  only (builder's choice).
- Gateway: clip posted, edited, deleted; the usual reaction and reply events.

## 7. Tests

- Client: the cut model (cut, drop, restore, edge drag, undo, kept-time sum, the 1:30 limit), the game guess from paths
  (NVIDIA folder, Desktop, unknown), the ffmpeg command built for each preset (no upscaling, fps cap), the encoder
  fallback (GPU missing → CPU), the pending card states (preparing, uploading, failed, retry, cancel).
- Server: signed upload/download URLs only for members, size and duration checks, confirm step rejects missing or
  oversized objects, game registry uniqueness, edit/delete by the poster only, deleting removes the object, storage
  totals.
- UI: list and wall render, toggle keeps position and is remembered, paging, filters, new markers, one-at-a-time
  playback, fullscreen.
- Manual on Windows: a real 2-minute NVIDIA replay (in the group's actual format, §3.1) → cut out the middle → Light → posted and playable by a friend
  within about a minute on home internet.

## 8. Left to the builder

- The ffmpeg build and how it's bundled (licence-compatible, with NVENC support), and its size impact on the installer.
- The filmstrip generation (thumbnails along the timeline, made locally).
- The player component (native `<video>` with custom controls is expected).
- Whether thumbnails go to disk or R2.
