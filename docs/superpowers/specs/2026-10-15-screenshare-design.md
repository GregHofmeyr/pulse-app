# Screen share: design

Date: 2026-10-15. Status: draft for review.

Inputs: `docs/superpowers/research/screenshare-deep-summary.md` (the decisions from research),
`screenshare-deep-crosscheck.md` (rulings and the S1-S17 spike checklist), and the brainstorm of 2026-10-15.
Spike results (2026-10-22): `screenshare-spike-results.md`; the sections it changed say so.

## 1. Goal

Let friends share a game or screen to the voice channel they are in, and let others watch. It has to beat Discord on
the three pains the group actually has:

1. **The streamer's game FPS tanks.** Target: at most 5% FPS loss at 720p60 (spike S2).
2. **Viewers see stutter** (freezes, choppiness). Target: no freeze of 2.5 s or longer at 1-5% loss (spike S4).
3. **The streamer's PC slows down after about 30 minutes.** Target: a flat 2-hour soak (spike S3), and numbers in
   every report if it happens anyway.

### Scope

- **v1:** streaming from **Windows only**, in server voice channels. Viewing from Windows.
- **Follow-up releases:**
  - viewing from Linux (needs a second video stack, §9);
  - "notify me when they go live" per friend;
  - streaming in DM and group calls, once calls exist. The design is room-agnostic, so this only needs the call UI to
    show the LIVE badge and Watch button.
- **Not this feature:** clips and recording (a separate feature, after object storage).

### Facts about the group that shaped this

- Everyone is on Windows 11. GPUs are all NVIDIA: RTX 2080 Ti, 4070 Ti Super, RTX 5050, probably a 3060. There may be
  an AMD GPU and an AMD CPU in the group.
- Upload is about 50 Mbps or more for everyone.
- Games: competitive FPS (Overwatch, Apex Legends, CS2), story games, and co-op party games.
- Audio software: SteelSeries Sonar and Turtle Beach so far.

## 2. Architecture

### Two LiveKit rooms per voice channel

| Room | Name | Contents | Who can enter |
|---|---|---|---|
| Voice | `<channel_id>` (unchanged) | Rust voice, microphone only | Voice members |
| Stream | `<channel_id>~s` | All streaming: video and stream audio | Voice members, and ghosts (§5) |

The stream room exists only while someone is live or watching. Keeping it separate is what makes ghost mode safe: a
ghost's ticket names only the stream room, so LiveKit itself keeps them out of the voice chat.

**Hardening of the voice room:** voice tickets become `can_publish_sources: [microphone]`. Today they allow
publishing anything.

### Participants in the stream room

| Identity | Runs in | Does |
|---|---|---|
| `<uid>#v` | JS (`livekit-client` in WebView2) | Publishes the screen video while live; subscribes to the video of watched streams |
| `<uid>#a` | Rust (a second, lightweight `Room` connection) | Publishes stream audio while live; subscribes to the stream audio of watched streams and plays it through Pulse's mixer |
| `g_<random>#v`, `g_<random>#a` | Same, for a ghost | Subscribe only |

- Each person has one JS participant and one Rust participant in the stream room. The one exception is a pop-out
  window, which briefly adds a second JS participant, `#v2` (§4.3).
- **`#v` connects when its owner joins voice** (a ghost's connects on first Watch), and `#a` connects on first use.
  - Spike S12: connecting to the stream room took 1.3-1.6 s at 173 ms RTT, and the first frame took another
    0.6-1.5 s. So a cold Watch took 1.95-2.85 s against the 1.5 s bar.
  - Pre-connected, Watch to first frame is the second number only.
  - The first cut on `screenshare-v1` still connects on first use; this is a follow-up.
- They leave when the person leaves voice, or, for a ghost, closes the last stream.
- The voice room keeps its plain `<uid>` identity. The server's webhook already ignores identities and rooms it can't
  parse, so the `~s` room gets its own handling (§4.4).

### Why it's split this way

- **Video in JS:** the Rust SDK has no hardware codecs on Windows (0.9.5), and Tauri IPC is too slow for frames.
  Chromium gives Media Foundation hardware encode and decode, simulcast and rate control.
- **Stream audio in Rust (Approach 1 from the brainstorm):** it plays through Pulse's mixer, so the echo canceller hears
  it and a viewer on speakers doesn't echo the game into voice. Every viewer, in voice or ghost, follows the same path.
  - The cost: audio and video come from separate connections, so nothing syncs them. Research estimated audio lands
    0-80 ms behind the picture.
  - Spike S9 measured audio arriving +32 and +68 ms behind the picture. That is a thin sample (n = 2, on one PC), but
    it is inside the 0 to +125 ms bar.
  - So v1 has no sync controller and only logs the offset, as the first cut does. If a two-PC rerun or the slowdown
    watch shows the offset outside the bar, the viewer delays video by the measured offset.
- **Voice is unchanged.**

### Step zero (a prerequisite, merged on its own)

Voice receive (`voice/mod.rs` `RxTasks`, `mixer.rs` `peers`) is keyed by participant identity. A second audio track
from the same person would replace the first. Key both by **track SID**, with the participant identity kept for
per-person volume and mute. This is test-first, with no behaviour change for voice.

### Server LiveKit configuration

- `enabled_codecs`: LiveKit's full default list **plus** strict H.264 entries for profile-level-id `4d001f` (Main) and
  `42001f` (Baseline), packetization-mode=1. Never trim the list: that drops voice RED and video RTX.
  - Reason: Chromium skips NVIDIA's encoder for Constrained Baseline (`42e01f`, LiveKit's default), crbug 1088650.
- UDP receive buffers to 16 MiB (sysctl), and LiveKit's buffer settings to match.
- The instance resized to 2 OCPU before launch. Planning load: 25-45% of one OCPU for 2 streams × 6 viewers at
  1080p60.
- LiveKit v1.13.10 or later (deployed 2026-10-15).

## 3. The streamer

### 3.1 Going live

1. **Go Live** button in the voice panel. It appears on Windows only. On Linux it's disabled with "Streaming from Linux
   isn't supported yet".
2. Pulse calls `getDisplayMedia`, and WebView2 draws its picker (Screens and Windows tabs) inside our window. The
   picker is unusable below 600 px tall and Pulse's minimum height is 560 px, so Pulse grows the window to at least
   640 px while the picker is open and restores it afterwards.
3. **The Go Live dialog:**
   - the picked source with a live preview, and **Change source** (reopens the picker);
   - **Resolution:** 720p, 1080p or 1440p (never above the source's size);
   - **Frame rate:** 15, 30 or 60;
   - **Stream audio:** on by default;
   - **Allow ghost viewers:** on by default;
   - **Go Live.**

   The last settings are remembered per device. The defaults the first time are 720p60.
4. **The first time** (and from Settings later): a short upload test against our server. It POSTs about 3 MB and takes
   the median of three runs. The bitrate cap is 70% of the result. Combinations whose bitrate exceeds the cap show a
   warning, not a block.

### 3.2 Encoder settings

These are pure functions of (resolution, fps, cap) and unit-tested.

| Resolution | 15 fps | 30 fps | 60 fps |
|---|---|---|---|
| 720p | 1.5 Mbps | 3.5 Mbps | 5.5 Mbps |
| 1080p | 2.5 Mbps | 5 Mbps | 8 Mbps |
| 1440p | 4 Mbps | 7.5 Mbps | 12 Mbps |

These are starting values; spikes S2 and S4 tune them. Each is clamped to the cap.

- **30 or 60 fps (games):** `contentHint = 'motion'` and `degradationPreference = 'maintain-framerate'`.
- **15 fps (text):** `contentHint = 'detail'` and `degradationPreference = 'maintain-resolution'`.
- **Simulcast:** the full layer plus one low layer, 360p at 30 fps and about 500 kbps, always. A single-layer screen
  share is exempt from LiveKit's stream allocator, so a slow viewer would simply freeze.
- Settings go in `screenShareEncoding` and `screenShareSimulcastLayers`. `livekit-client` ignores `videoEncoding` for
  screen share.
- **Codec:** H.264, with the client preferring Main (`4d001f`). Baseline `42001f` is fine too: the spike negotiated it
  and the NVIDIA MFT encoded it (S1). The one profile to avoid is Constrained Baseline `42e01f`. AV1 is not in v1: the
  2080 Ti can't decode it in hardware.
- **Encoder check:** 5 s after publishing, and every 30 s after that, read `encoderImplementation` and
  `powerEfficientEncoder` from outbound-rtp stats. If it isn't hardware, the card shows "Your GPU encoder isn't being
  used, so this will cost game FPS" and the event is logged.
  - **Judge the full layer.** With simulcast, the string is one `SimulcastEncoderAdapter (…, …)` listing every
    layer, lowest first.
  - In S1 the low layer of a near-square window ran on OpenH264 for its first 25 s, then moved to the MFT. A software
    low layer is only logged.
  - Warn only when the full layer isn't hardware on two reads in a row.

### 3.3 While live

- **The "You're live" card:**
  - a small preview of the local capture (no round trip through the server), drawn only while Pulse's window is
    focused, with a **Hide preview** toggle that is remembered;
  - names of in-voice viewers and the 👻 count;
  - current resolution and fps;
  - the health dot (§6.3);
  - **Stop**, **Mute stream audio** and **Settings**.
- **Focus comes from Rust window events, not `visibilityState`.** Tauri doesn't mark the page hidden when the window is
  minimised (#10592).
- **Settings change live, without republishing:**
  - bitrate and fps through `RTCRtpSender.setParameters`;
  - resolution through `applyConstraints`;
  - a new source through `replaceTrack` on the same publication.

  One publication lasts the whole stream. Research saw about 20 MB leaked per unpublish and republish cycle.
- **Pulse's animated UI freezes while live** (a `streaming` class turns off CSS animations and pauses animated avatars
  and emoji). It was measured at about a core otherwise.
- **Stream audio** (Rust, the `wasapi` crate's process loopback):
  - **Window share:** INCLUDE mode on the shared window's process tree.
  - **Screen share:** EXCLUDE mode on Pulse's own process tree, when Pulse is the only Pulse running.
  - **Another Pulse instance running** (a second profile): EXCLUDE takes only one process tree, so it would capture
    the other instance's playback. Spike S9 fed back exactly that way. Instead, INCLUDE every process with an
    active audio session that isn't a Pulse process. The first cut's `stream/plan.rs` does this.
  - If the shared window's process can't be identified, a window share uses the screen-share rules.
    - Spike S8 found the track label is `window:<id>:0`, never the window title, so the label never identifies it.
    - The process has to come from the window handle, or not at all.
  - Known gap: EXCLUDE also captures other voice apps (a Teams or Discord call running beside Pulse). S9 ran with a
    Teams call open. Nothing in v1 filters them.
  - Stereo Opus at 48 kHz, about 96-128 kbps. This needs the rust-sdks #1018 stereo patch.
  - **Mute stream audio** stops sending audio without ending the stream.

### 3.4 Stopping

The stream stops on **Stop**, when the streamer leaves voice, or when the shared window closes (the track's `ended`
event). The client unpublishes, Rust stops the audio connection, and the server broadcasts `StreamEnded`. Viewers see
"<name> stopped streaming".

## 4. The viewer

### 4.1 Finding a stream

- A red **LIVE** badge on the streamer's name in the voice list, the channel list and the "Your people" board.
- Hovering or clicking it opens a popover:
  - the latest preview image, fetched when hovering;
  - resolution and fps;
  - the viewer count, including 👻;
  - **Watch**. The button reads "Join voice to watch" when the viewer isn't in voice and the stream doesn't allow
    ghosts.
- **The live toast:** when someone goes live, everyone *in that voice channel* gets "<name> is live" with **Watch**,
  dismissed after 4 s. Nobody else is notified (they see the badge).

### 4.2 Previews

- While live, the streamer's client draws the capture into a 480×270 canvas every 15 s, encodes it to WebP at quality
  0.7, and uploads it.
  - Endpoint: `PUT /streams/{channel}/preview`, about 20-40 KB, live streamers only.
- The server keeps only the latest image per stream, in memory, and drops it when the stream ends.
- Viewers fetch it with `GET /streams/{channel}/{user}/preview`. Server and channel membership is checked, and ghosts
  are allowed if the stream allows them. The response carries an ETag.

### 4.3 Watching

- The stream takes over the main area, with chat collapsed to a side strip. Two or more streams sit side by side;
  clicking one makes it the large one.
- **Each tile has:**
  - **Volume** for stream audio (separate from voice), remembered per streamer;
  - **Fullscreen**;
  - **Pop out**;
  - **Quality:**
    - **Auto** (default): the full layer, dropping to the low layer only when the viewer's connection can't keep up
      (the SFU's allocator decides);
    - **Data saver:** the low layer only (`setVideoQuality(LOW)`);
  - **Stats overlay** (hidden by default): resolution, fps, bitrate, round-trip time, loss, and the health wording
    (§6.3).
- **Subscription settings:**
  - `adaptiveStream: false`, so tile size never lowers quality behind the viewer's back;
  - `dynacast: true`;
  - the viewer's SDP answer carries `rtx-time=300` on the RTX codec;
  - `<video>` with hardware decode.
    - Pulse can't check it in-app: Chromium leaves `decoderImplementation` and `powerEfficientDecoder` out of
      inbound-rtp stats, even after a `getUserMedia` call (S7).
    - Task Manager showed Video Decode at about 40 % for 1440p55 on a 2080 Ti.
- **Upscaling:** when the decoded size is smaller than the tile, a WebGL pass draws it with Lanczos or FSR1, never the
  browser's bilinear scaling. Text streams (15 fps) are never downscaled.
- **Pop out:** opens a new Tauri window with its own page.
  - The new page subscribes to the stream (as a second `#v` connection, `<uid>#v2`), and the main window unsubscribes
    once the pop-out's first frame arrives. There's a short overlap and a blip of about 1 s; still one decode at a time.
  - Closing the pop-out returns the stream to the main window the same way.
  - Spike S18 measures the blip.
- **Minimised:** streams in the main window pause after the window has been minimised for 5 s (the tracks are
  unsubscribed, and stream audio is paused too). They resume on restore. A pop-out window follows its own minimised
  state.
- **Stopping watching:** close the tile, or leave voice; a ghost just closes the last tile. The server updates the
  viewer list.

### 4.4 Server state and events

The server keeps live-stream state in memory, per channel:

- **Per stream:** streamer, settings (resolution, fps, audio, allow-ghosts), status (`live` or `reconnecting`), named
  viewers, and the ghost viewer set (opaque IDs mapped to user IDs, server-side only).
- **Gateway events,** to everyone who can see the channel:
  - `StreamStarted`;
  - `StreamUpdated` (settings or status);
  - `StreamEnded` (with a reason: `stopped`, `left_voice`, `crashed`, `timed_out`);
  - `ViewersChanged` (named viewers and the ghost count).
- **Sources of truth:**
  - client calls: go live, update, stop, watch, unwatch;
  - the `~s` room webhook: participant joined and left;
  - the gateway connection (§7).

  When they disagree, the webhook wins for "who is connected".

## 5. Ghost mode and permissions

### 5.1 Rules

| Action | Allowed when |
|---|---|
| Go live | In that voice channel now, on Windows |
| Watch as yourself | In that voice channel |
| Watch as a ghost | A member of the server, not in that voice channel, and the stream allows ghosts |
| Hear voice | Only with a voice-room ticket; ghosts never get one |

### 5.2 Tickets

- **`POST /streams/{channel}/live`:** the go-live ticket for `<uid>#v`, with `can_publish_sources: [screen_share]`
  and subscribe allowed.
- **`POST /streams/{channel}/live/audio`:** the ticket for `<uid>#a`, with
  `can_publish_sources: [screen_share_audio]` and subscribe allowed.
- **`POST /streams/{channel}/watch`:** subscribe-only tickets (`can_publish: false`, `can_publish_data: false`) for
  `#v` and `#a`.
  - A voice member gets `<uid>#v`/`#a`.
  - A ghost gets `g_<random>#v`/`#a`: 16 random bytes, base32, new for every watch session, never derived from the
    user ID.
  - A pop-out asks for a `#v2` ticket of the same kind (`<uid>#v2` or `g_<random>#v2`, reusing the session's random
    part).
- All tickets are scoped to room `<channel_id>~s` and last 10 minutes. The client refreshes them; LiveKit only checks a
  ticket at connect.
- The watch endpoint is rate-limited, like the other ticket endpoints.

### 5.3 Anonymity and accountability

- Ghost identity never leaves the server. Even the streamer sees only the 👻 count.
- The server log records `ghost watch` lines (user, channel, opaque ID) for the admin. They are never stored in the
  database and never sent to any client.

### 5.4 Changes mid-stream

- **Allow ghosts switched off:** the server removes every ghost participant from the room (LiveKit
  `RemoveParticipant`). Their clients show "<name> switched to voice-only".
- **A ghost joins voice:** their client drops the ghost connections and reconnects as `<uid>#v`/`#a`. One 👻 becomes
  a name.
- **Someone leaves or is removed from the server:** the server removes their stream-room participants.
- **The streamer leaves voice:** the stream ends (`left_voice`).

## 6. Adaptation and health

### 6.1 Network (built in)

- WebRTC congestion control plus the degradation preference (§3.2).
- The SFU picks each viewer's layer.
- Dynacast pauses layers nobody watches.

### 6.2 Performance step-down (Pulse, on by default)

- **Every 2 s, Pulse reads:**
  - sent fps against the target;
  - capture frames dropped (`MediaStreamTrack` stats, or `framesDropped` from the encoder);
  - encode time per frame (`totalEncodeTime / framesEncoded`);
  - `qualityLimitationReason == 'cpu'`.
- **Strain** means any of these for 3 samples in a row (6 s):
  - sent fps below 85% of the fps the capture delivered. Spike S21 compared against the target instead, and `lowFps`
    fired on every sample of a still screen (capture drops to about 2 fps when nothing changes) and of a game that
    only rendered about 38 fps. Neither is the encoder struggling. A low capture rate alone is "still" or
    "source-limited", never strain;
  - capture drops above 5%;
  - encode time above 70% of the frame interval;
  - CPU-limited.
- **The ladder:** 60 → 30 fps, then 1440p → 1080p → 720p. The floor is 720p30. One rung per strain episode, then a 20 s
  cool-down.
- **Each step shows a toast:** "Lowered to 30 fps to keep your game smooth. **Undo**". Undo restores the previous
  setting and turns auto step-down off for the rest of this stream.
- **There is no automatic step-up.** The card shows "Lowered to 30 fps, **try 60 again**".
- **Off switch:** Settings → Streaming → "Lower quality automatically when my PC struggles" (on by default).
- The state machine is a pure, unit-tested function. Spike S21 found the sent-vs-target signal wrong (above). Whether
  the corrected signals fire before the game visibly stutters still needs a two-PC run; the first cut ships no
  step-down.

### 6.3 Health

- Our own rules: LiveKit's connection quality and `qualityLimitationReason` said "fine" during real trouble.
- **Streamer:**
  - 🟢 healthy;
  - 🟠 or 🔴 with a cause: **"your upload"** (bitrate held under 70% of target for 10 s, rising round-trip time, or
    loss above 2%) or **"your PC"** (the strain signals in §6.2).
- **Viewer:** the same dot in the stats overlay. **"Your connection"** means receive-side loss, jitter or a frozen
  picture with the streamer healthy.
  - A still stream isn't frozen. WebRTC's `freezeCount` counted 31 freezes (23 s) in a minute of a still screen, with
    0 loss (S4/S9).
  - "Frozen" needs loss, NACKs or PLIs alongside it, or a streamer whose capture is producing frames.
  - The first cut's overlay already shows a still screen neutrally (`fpsNote 'still'`).
- **Viewer, "the streamer":** the streamer's `StreamUpdated` health is amber or red.
  - The streamer's client sends its health to the server every 10 s and on every change. The server includes it in
    `StreamUpdated`.
- The rules are pure functions, tested against recorded stats fixtures.

### 6.4 The slowdown watch

- While streaming or watching, a `share:` log line every 5 s with:
  - Pulse's processes' working sets (the main process and every WebView2 process) and GPU dedicated and shared memory;
  - Pulse's handle count;
  - the Windows non-paged pool;
  - the working sets of `dwm.exe` and `audiodg.exe`;
  - capture fps, dropped frames, encode ms, bitrate per layer, round-trip time and loss.
- **The trend check:** a least-squares slope over a sliding 10-minute window. A sustained climb (any memory figure
  growing more than 50 MB per 10 min, or handles more than 500 per 10 min) logs a warning once per metric. No UI
  interruption.
- Bug reports attach the last 10 minutes of `share:` lines and any warnings automatically.
- Logging stays generous while we're testing.

## 7. Failure handling

| What | Behaviour |
|---|---|
| The streamer's network drops | LiveKit resumes for about 50 s, then gives up for good. Pulse's own reconnect loop retries (1, 2, 4… capped at 30 s) for up to 5 min while still in voice. Viewers see the frozen last frame with "Reconnecting…". The server marks the stream `reconnecting` and ends it after 60 s with no media (`timed_out`). |
| The streamer's WebView2 renderer crashes | Rust gets `ProcessFailed`, tells the server, and the server sends `StreamEnded{crashed}` (viewers know within about 1 s). Voice keeps working. Rust reloads the page, shows "Stream crashed, **Go Live again?**", and files a crash report. |
| The streamer's whole app dies | The gateway socket closes at once. The server marks the stream `reconnecting`, then `StreamEnded{crashed}` after 15 s without a reconnect. |
| A viewer's renderer crashes | Rust reloads the page and re-subscribes to the same streams. Voice is untouched. |
| The shared window closes | The track `ended` event stops the stream (`stopped`). |
| The shared window is minimised, or a game in exclusive fullscreen captures black | Detected as no new frames, or all-black frames, for 3 s. The card says "Your shared window is minimised / can't be captured. Try borderless fullscreen." Spike S20 checks the group's games. |
| The GPU encoder drops out mid-stream | The encoder check (§3.2) sees the implementation change and shows the warning. |
| Stream audio fails | Video continues. The card shows "Stream audio unavailable, retrying"; Rust retries with backoff. |
| A server restart or deploy | Clients that are live re-send their stream state when the gateway reconnects (as voice reconciles today), so streams reappear within seconds. Viewers re-request watch tickets. |
| The upload test or a ticket request fails | A clear error with **Retry**; never a silent no-op. |

## 8. Order of work and testing

### 8.1 Order

1. **Step zero:** per-track keying in the voice receive and mixer paths (§2). Test-first, merged alone.
2. **Windows spike (about 2 days, throwaway).**
   - A spike build with a hidden debug panel (one button per test, automatic logging into bug reports). The owner runs
     it on the 2080 Ti PC and, ideally, the 4070 Ti Super PC.
   - Tests: S1-S17 from the cross-check, plus:
     - **S18:** the pop-out handover blip;
     - **S19:** game audio to voice echo with speakers;
     - **S20:** exclusive fullscreen versus borderless capture in Overwatch, Apex Legends and CS2 (anti-cheat
       included);
     - **S21:** do the step-down signals fire before the game visibly stutters?
   - Results that change this spec (hardware encode fails, FPS cost above 5%, audio sync above about 80 ms) go back to
     the owner before building.
3. **Build, in phases.** A review at the end of each phase.
   1. **Server:** stream rooms, tickets and permissions, ghosts, state, events, webhook, previews, the voice-ticket
      hardening, LiveKit codec config.
   2. **Streamer:** Go Live, the dialog, encoding, the encoder check, the card, live settings changes, stream audio
      capture and publish.
   3. **Viewer:** badge, popover, toast, tiles, quality, pop-out, minimise pause, upscaling, stream audio playback.
   4. **Adaptation, health and the slowdown watch.**
   5. **Failure handling and polish.**
4. **A security review of ghost mode** before merging.
5. **A group Windows test,** including a 2-hour stream in a real game.

### 8.2 Tests

- **Server (TDD):**
  - every rule in §5.1;
  - a ghost never receives a voice-room ticket;
  - a ghost identity never contains or derives from the user ID;
  - allow-ghosts off removes ghosts;
  - leaving the server removes participants;
  - state reconciliation after a restart;
  - every gateway event and its audience;
  - preview access rules.
- **Integration (real LiveKit in Docker, like the existing voice tests):** a stream-room participant **cannot**
  subscribe to any voice-room track. This is the most important test in the feature. Also: a watch ticket can't
  publish, and a `#v` ticket can't publish audio.
- **Rust:**
  - per-track mixer and receive keying;
  - stream audio reaching the mixer and the echo canceller's far-end reference;
  - the lifecycle of the second connection (connect on first use, leave with voice, ghost to named switch);
  - loopback INCLUDE and EXCLUDE selection.
- **JS (pure functions, unit-tested):**
  - settings to encodings;
  - the step-down state machine (including Undo locking it);
  - the health rules on recorded stats;
  - the watch, pop-out and ghost state machine;
  - the trend check.
- **A manual Windows checklist:** the picker and window growth, the hardware encoder on each GPU, games and anti-cheat,
  minimise and focus, pop-out on a second monitor.

## 9. Later

- **Linux viewing:** WebKitGTK has no usable WebRTC. The plan is Rust decode → localhost WebSocket → WebGL2 (spike S17,
  about a week).
- **Notify me when <friend> goes live.**
- **Streaming in DM and group calls,** after calls ship.
- **AV1, opt-in,** when every viewer's GPU can decode it.
- **A native zero-copy capture path** if S2 shows Chromium's capture costs more than 5% game FPS.
- **The Rust pass-through viewer** (rust-sdks #1515) if it merges and helps.
- **Clips and recording,** a separate feature after object storage.
