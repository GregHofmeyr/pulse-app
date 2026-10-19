# Screen share: the viewer and streamer experience, and making it cheap

**Date:** 2026-10-14 · **Scope:** what people see and do around a screen share ("Go Live") in Pulse, and how to keep it cheap
in bandwidth, CPU and GPU. It builds on the four reports next to this file (summary, render, capture-encode, server-quality),
which settled the plumbing: screen-share **video** runs in WebView2 through `livekit-client` (a second, hidden participant
per user), **voice and stream audio** stay in Rust.

Read-only research. Nothing in the app was changed. Throwaway benchmarks live in the session scratchpad, not the repo.

**Labels used on every claim:**

- **[cited]**: a source says so. The source ID (S-number) maps to a URL and the date it was read in §11.
- **[measured]**: I measured it for this report (method in §11, M1).
- **[est.]**: my estimate from the cited numbers.
- **[speculation]**: a reasoned guess that the spike must confirm.

---

## 0. TL;DR

1. **Copy Discord's model, not its rough edges.** Opt-in "Watch", a LIVE badge in the channel list, a refreshed preview
   picture for people who aren't watching, focus and grid views, full screen, a pop-out, per-stream volume, and
   "stop watching" are all things Discord users expect [cited S1, S2]. The common complaints are: you must join voice to
   watch, the preview is stale (it changes "every few minutes"), there's no viewer-side quality choice, stream audio is
   too quiet or gets ducked, and the pop-out window sometimes vanishes [cited S8, S13, S14, S15, S12].
2. **Previews should be pictures, not video.** A 480×270 WebP every 15 s is a median **13 KB** for game frames
   (p90 32 KB) and **3 KB** for desktop and text, and costs about 29 ms to encode in Chrome [measured M1]. That is about
   **10 kbps** per person looking at it, against **200-2,000 kbps** for even the smallest live video layer, so it is
   **20-150× cheaper** in bandwidth, with no decoder, no keyframe request and no extra encoder layer on the streamer [est.].
   Send it over our own HTTP server plus a gateway event, not LiveKit.
3. **One window, three sizes, is the cheap default.** Inline tile, focus (fills the content area) and full screen all use the
   same `<video>` and the same subscription. For "keep watching while I do something else", shrink the **main window** into
   an always-on-top mini player. That costs nothing extra. A true second pop-out window is possible through `window.open`
   and Tauri's `on_new_window`, which keeps one LiveKit connection, but it needs a spike [cited S37, S38; speculation].
4. **Two WebView2 traps to design around:**
   - `element.requestFullscreen()` only fills the webview, not the screen. JS must also call the Tauri window's
     `setFullscreen` [cited S39].
   - **Minimising Pulse does not pause video.** wry never tells WebView2 the window is hidden, so
     `document.visibilityState` stays `"visible"` and `livekit-client`'s background pause never fires [cited S34, S35].
     Pulse must pause the subscription itself on minimise.
5. **`livekit-client` adaptive stream works, with two caveats** [cited S30, S31]:
   - it reacts to tile size and to tiles that are scrolled away or hidden;
   - by default it counts CSS pixels, not device pixels, below 3× scaling, so it under-asks on 125-200% displays. With our two
     layers (720p fallback, 1080p+ top) this rarely matters [est.], but set `pixelDensity: 'screen'`.
6. **Render quality is mostly free:**
   - Chromium promotes `<video>` to a hardware overlay when nothing fancy is drawn over it [cited S43];
   - RTX/Intel video super resolution happens in the same overlay path [cited S42].
   - The UX rule that follows: no blur or transparency effects over the video, no CSS transforms on it, and controls that hide
     themselves.
   - Leave `<video>` in place. WebCodecs plus canvas adds work and loses those benefits for little latency gain [est.].
   - A "low-latency mode" exists: Chromium renders a frame immediately when the playout-delay header says min = 0. The
     LiveKit server can set that room-wide, which is worth one spike experiment [cited S44, S45, S32].
7. **Foolproof defaults** for non-technical friends:
   - pre-select "share a window", and warn before a whole screen;
   - hide Pulse itself from the capture while live, using `setContentProtected(true)` → `WDA_EXCLUDEFROMCAPTURE` [cited S28, S29];
   - mute Pulse's own pop-ups while live;
   - label quality by purpose ("Smooth for games", "Sharp for text");
   - show a big red "You're live · 2 watching" bar with Stop and Pause;
   - make "Watch" always an explicit click.
8. **v2 ideas**, in order of value-for-effort: reactions over the stream, a remote pointer and "draw on screen"
   (Slack does drawing [cited S22]), clips from the streamer's encoded ring buffer, co-watch, and a custom Pulse picker
   with thumbnails once Pulse does its own capture (Phase B in capture-encode).

---

## 1. What Pulse looks like today (ground truth)

From the UI code [cited S52]:

- **Layout.** A 52 px header with server tabs. A 264 px left panel holds the channel list, a "Voice connected" card
  (`.vc`) and the "me" bar (mute, deafen, settings). A rounded content panel sits in the middle, and the member list is on the right.
  The look is dark layered greys (`--bg-0..4`), an indigo accent (`--accent #8b9cff`), red danger (`--danger #d93a42`) and
  green ok (`--ok #4fd18b`). The font is Onest.
- **Channel list.** Voice channels list their occupants as indented rows: 22 px avatar, name, mute/deafen flags, and a green
  ring when speaking. **This is where a "LIVE" pill and a hover preview naturally go.**
- **Voice panel.** A grid of 220 px-min tiles, 200 px tall. Each holds an avatar, name, connection bars and a volume popover.
  A control dock sits at the bottom (mute, deafen, leave). **Stream tiles and the "Share screen" button go here.**
- **Keybinds.** Global keybinds (`toggle_mute`, `toggle_deafen`) are recorded by physical key and registered in Rust. A
  "start/stop stream" bind could reuse this.
- **CSP.** `img-src 'self' data:`. A preview image must be a `data:` URL, or `blob:` must be added. All HTTP goes through Rust
  (`connect-src ipc:`), so preview fetches go through the Rust API client like every other call.
- **Server.** Axum routes plus a gateway hub with audiences. `reports` already shows an upload route with a
  `DefaultBodyLimit`, which is the pattern for a preview upload.
- **Gateway events.** `VoiceJoined`, `VoiceLeft` and `VoiceStateChanged` exist (with `VoiceFlags`). A streaming flag or a
  `StreamStarted`/`StreamEnded` event fits beside them.

---

## 2. Discord Go Live, catalogued

All [cited] rows come from Discord's own help articles (S1-S6, updated 2026-10, read through the Zendesk API) unless noted.

### 2.1 Feature by feature

| Area | What Discord does | Source |
|---|---|---|
| **Start** | Join a voice channel, then click the screen icon in the voice status panel (bottom left). A window opens: pick an application window or a screen, then **Go Live**. A second route: while a recognised game runs, a "Stream <game>" button lets you pick the voice channel and quality in one dialog. A third: from the in-game overlay. | S1 |
| **Game detection** | Streaming "the game" relies on Discord's game detection. Unrecognised games can be added by hand. | S1 |
| **Source picker** | Separate Applications and Screens choices with thumbnails. "Change Window" switches source without stopping the stream. | S1, S2 |
| **Quality picker** | Resolution and frame rate, set at start or changed live from the stream's gear menu or the in-call arrow menu. Free accounts get up to 720p30, Nitro up to 4K60. The browser client can't change quality. | S1, S2 |
| **Hide preview** | Per stream, a "Hide stream preview" checkbox in the start dialog. Globally, a "Stream Previews" toggle in Voice & Video settings. When on, server members can see a preview of the stream before joining it. | S1 |
| **Notify friends** | Optional "Notify my friends in this server" checkbox (servers under 50 members). Rate-limited to one per 15 min for the sender and one per 6 h for each recipient. | S5 |
| **LIVE badge** | A red "LIVE" badge next to the streamer's name in the voice channel list. | S1 |
| **Preview picture** | Shown to people who aren't watching (on hover or in the call view). A user describes it as "the automatic one changing every few minutes". | S1, S8 |
| **Watch** | Double-click the streamer's name, or click it and choose **Watch Stream**. You must be in that voice channel, and joining another channel drops the stream. | S1 |
| **Multistream** | The eyeball icon → "Start watching multiple streams". "Watch Stream" instead switches focus to the new stream. | S2 |
| **Views** | **Grid** (default) and **Focus**: one big stream with the others in a strip, plus "Hide Members" to show only the main stream. "Show Non-Video Participants" can be unticked to hide audio-only people. | S1, S2 |
| **Window modes** | **Pop Out** to a separate resizable window with **Stay On Top**, and **Full Screen**. | S1 |
| **Streamer self-view** | A small picture-in-picture window of your own stream. Hover it for settings or to end the stream. It **pauses automatically when the game has focus**. | S1 |
| **Per-stream volume** | Right-click the stream → **Mute** and a **Stream Volume** slider. Separate from the person's voice volume. | S1, S2 |
| **Stop watching** | In the stream's options menu, or the in-call arrow menu ("which streams you'd like to stop watching"). | S2 |
| **Viewer feedback for the streamer** | In the overlay, an eyeball icon next to the names of people watching. | S1 |
| **Clips** | For Nitro (early access, Windows only, GPU minimums). **Alt+C** saves the last 30 s, 60 s or 2 min of your stream to disk. You can then trim it and share it into chat. | S3 |
| **Capture method (Windows)** | An injected DLL by default. On Windows 11 the default is **Windows Graphics Capture**, which "does not work for full screen exclusive games". They detect fullscreen-exclusive with `SHQueryUserNotificationState` and fall back. They tell users to pick **Borderless**. | S4 |
| **Streamer Mode** | Hides personal info, invite links, notifications and sounds. Can switch on automatically when OBS/XSplit runs. Has a keybind. | S6 |
| **Limits** | 50 viewers per stream, and 50 video/screen shares per voice chat. | S1 |
| **Engineering promises** | Video is relayed only to people watching. The streamer transmits only if at least one viewer watches. The stream is not synced with voice. "The streamer will not transmit more data than what the slowest connection can support." | S7 |

### 2.2 What users praise and what they hate

Praise is implied by vote counts more than stated: the pop-out request had **5,980 votes** before it shipped
[cited S11]. Complaints, with sources:

| Complaint | Evidence | Lesson for Pulse |
|---|---|---|
| Must join voice to watch ("awkward silence") | 72 votes [cited S13]; a petition of about 1.5k signatures says clicking a stream moves you into voice without warning [cited S53] | For 6 friends, joining voice to watch is fine. But never **move** someone into voice by surprise: "Watch" on a channel you're not in should say "Join voice and watch". |
| Preview is stale | "changing every few minutes" [cited S8] | Refresh every ~15 s; it's cheap (§4). |
| No viewer quality choice | Request for 720p/480p/360p viewing options [cited S14] | Simulcast lets a viewer pick "Data saver" without hurting others (§5.6). |
| Stream audio too quiet, or louder only after someone speaks (ducking), or level tied to the streamer's own volume | [cited S15] | Stream audio needs its own bus with no voice processing and a per-stream slider. Capture pre-volume if possible. That was already planned in Rust [cited S51]. |
| Quality drops ("reduced, may be a network problem") on good connections | [cited S15, S17] | Show the real reason (streamer upload vs your download) in plain words. One slow viewer should not drag everyone down (simulcast) [cited S7, S51]. |
| Free tier is 720p30 and looks bad | [cited S16, S1] | No tiers. Default 720p60 for games and 1080p30 for text [cited S51]. |
| Pop-out window disappears (taskbar only; Win+D brings it back) | [cited S12] | If we do pop-out windows, clamp them to a visible monitor on open and on display changes. |
| "Go Live" nag popping up every minute in some games | 101 votes [cited S9] | Never nag. No "you could stream this" prompts. |
| Wanted: request control, draw on screen | [cited S18] | v2: pointer and drawing (§9). |

---

## 3. How the others do it

| Product | Notable UX | Source |
|---|---|---|
| **Microsoft Teams** | A **red border** around whatever you share. An "Include sound" toggle. A presenter toolbar at the top of the screen (stop, give control, presenter modes) | S19 |
| **Zoom** | What participants see is **outlined in green**. "Share sound". "Optimize for video clip", which makes still content blurry if left on, and greys out Zoom's own windows that overlap the shared area. Users report Zoom silences Windows notifications while sharing | S20, S21 |
| **Google Meet / Chrome** | Recommends sharing a **window or tab, not the whole screen**, to avoid the "tunnel" (hall of mirrors) effect. Since Chrome 88, Chrome **hides the content of web notifications while you share your screen**, with "Show all" to reveal them | S23, S24 |
| **Slack huddles** | Up to **two people share at once**. Anyone can enlarge a share or **draw on it** with a pencil (desktop apps). Temporary emoji reactions, and sticky "stickers" such as raise hand | S22 |
| **Guilded** | Was the closest Discord clone with free high-quality streaming. Roblox **shut it down at the end of 2025** | S25 |
| **Chromium `getDisplayMedia` controls** (what our picker can ask for) | `displaySurface` pre-selects the Windows/Screens/Tabs pane (Chrome 107). `monitorTypeSurfaces: "exclude"` removes whole screens (119). `selfBrowserSurface: "exclude"` hides the current tab. `surfaceSwitching` adds "Share this tab instead". `systemAudio` controls system audio (105) | S26 |

What we take:

- the **border** (Teams/Zoom) as a privacy cue;
- **window over screen** (Meet);
- **muted notifications** while sharing (Chrome/Zoom);
- **drawing and reactions** (Slack) for v2.

---

## 4. Cheap previews

### 4.1 What the preview is for

People in the server who aren't watching should see *what* is being streamed: "Alex is playing Rocket League", or "Sam has
VS Code open". That helps them decide to click Watch. It needs to be recognisable, not smooth. Discord gets away with a
refresh "every few minutes" [cited S8].

### 4.2 Design

**Streamer side** (the JS publisher that already holds the capture `MediaStreamTrack`):

1. About 1 s after going live, and then every **15 s**, take a frame. Use `new ImageCapture(track).grabFrame()`, which
   gives an `ImageBitmap` [cited S49], or draw the local preview `<video>` into a canvas.
2. Draw it into a **480×270** `OffscreenCanvas` with high-quality smoothing.
3. Encode it with `convertToBlob({type: 'image/webp', quality: 0.6})`, ideally in a Worker.
4. Skip the upload if it is byte-identical to the last one (a paused or static screen).
5. `POST /voice/{channel}/streams/me/preview` (≤ 64 KB, authenticated, only while the user is in that channel and
   streaming).

**Server:** keep only the **latest** image per stream, in memory, with a revision number. Broadcast a tiny gateway event
`StreamPreview { channel_id, user_id, rev }` to the server's audience. Delete it when the stream ends.

**Viewers:** fetch `GET .../preview?rev=n` through the Rust API client, and **only when the preview is on screen**: the
hover card on the channel list, or the stream tile in the voice panel for someone not watching it. Show it as a `data:` URL
(fits the current CSP) or add `blob:` to `img-src`.

**Phase B** (Rust-side capture, see capture-encode): the same protocol. Rust downscales on the GPU and encodes WebP.

**Why not the LiveKit data channel?** It would push every image to every participant, watching or not. The hidden `#v`
connection only exists while watching, so non-watchers would need the Rust connection to receive it. And people not in the
voice channel couldn't see previews at all. The HTTP route is pull-on-demand, permission-checked, and reuses the gateway
[est.].

### 4.3 What it costs [measured M1, est. where noted]

| Content | Size | Format | Median | p90 | Max |
|---|---|---|---|---|---|
| Games (17 frames, 1080p screenshots of free games) | 480×270 | WebP q0.6 (Chrome canvas) | **12.9 KB** | **31.9 KB** | 34 KB |
| | 480×270 | JPEG q0.75 (Chrome canvas) | 22.1 KB | 39.2 KB | 41 KB |
| | 640×360 | WebP q0.6 | 19.0 KB | 50.1 KB | 51 KB |
| | 320×180 | WebP q60 (libwebp) | 7.4 KB | 13.9 KB | 16 KB |
| Desktop/apps/text (30 dev-machine screenshots) | 480×270 | WebP q60 (libwebp) | **2.7 KB** | 6.5 KB | 7.5 KB |

Encode time per snapshot (headless Chrome 151, software, i9 laptop CPU, including the downscale): WebP 480×270
**p50 29 ms, p90 47 ms**; JPEG 8 / 16 ms [measured M1]. Once every 15 s in a worker, that is noise [est.].

**Preview vs subscribing to a low simulcast layer** (per person looking at one stream) [est., from S30 presets and S51 layers]:

| | Snapshot every 15 s (480×270 WebP) | Low layer 360p 3 fps | Low layer 360p 15 fps | Our planned fallback layer, 720p 15-30 fps |
|---|---|---|---|---|
| Download | ~7 kbps median, ~17 kbps p90 | ~200 kbps (`h360fps3`) | ~400 kbps (`h360fps15`) | ~1,500-2,000 kbps (`h720fps15/30`) |
| Per hour | ~3 MB | ~90 MB | ~180 MB | ~675-900 MB |
| Viewer GPU/CPU | one image decode / 15 s | a live hardware decoder + 3-15 fps compositing | same, more | same, more |
| Streamer cost | one tiny encode / 15 s | dynacast keeps that layer **encoding the whole time** anyone previews | same | same |
| Keyframe on open | none | a PLI to the streamer each time a tile opens (a bitrate spike for every viewer of that layer [cited S51]) | same | same |
| Server | ~32 KB RAM per stream, trivial egress | forwards packets continuously | more | more |

So a snapshot is **roughly 30-250× less data** than a live low layer. It keeps the streamer's encoder free of an extra
always-on layer and needs no special simulcast layer just for thumbnails [est.]. A live 360p3 layer also looks worse than
a sharp still: 3 fps video stutters and its frames are blurry, while the still is a clean frame.

### 4.4 Privacy and controls

- **"Hide preview"**, per stream in the start dialog, with a global default, as Discord does [cited S1]. When hidden, the
  tile shows the app name and icon (if known) and "Preview hidden".
- **Pausing the stream** stops snapshots too.
- **Whole-screen shares:** the first snapshot waits until the stream has run for 3 s, so a notification that was on
  screen when you pressed Go Live is less likely to be the picture that sits in everyone's sidebar [speculation: small
  benefit; a cheap rule].
- **The streamer's own "your stream" card shows the same snapshot others see,** so you know exactly what your friends are
  looking at.

---

## 5. Viewing modes and what they cost

### 5.1 Modes

| Mode | What it is | Extra cost over watching | Notes |
|---|---|---|---|
| **Tile** | A stream tile in the voice-panel grid (~220-480 px wide) | none | adaptive stream asks for a smaller layer [cited S30] |
| **Focus** | The stream fills the content panel. Voice people show as a strip of small avatars, chat stays one click away | none | the default after clicking Watch |
| **Full screen** | Fills the monitor | none | needs a Tauri `setFullscreen` call (§5.3) |
| **Mini player** | The **main Pulse window** shrinks to a small, frameless, always-on-top player (e.g. 480×270, draggable, resizable) | **none**: same window, same `<video>` | Discord's pop-out plus stay-on-top, without a second window. You lose chat while mini; "Expand" restores it |
| **Pop-out window** (v1.5) | A second OS window with only the stream | a second top-level WebView2 window: ~20-60 MB memory [est.]; no second connection *if* it is opened with `window.open` | §5.4 |
| **Picture-in-Picture** (browser PiP) | the OS-level floating video window | unknown in WebView2 | §5.5. Don't depend on it |

### 5.2 Watching only what's watched

- `autoSubscribe: false`, and subscribe on **Watch**. This was already planned [cited S51].
- **Stop watching** → `setSubscribed(false)`.
- Watching more than one stream is allowed: each extra stream is a second hardware decode plus its download. Show
  "Watching 2 streams" in the header of the voice panel.
- Leaving voice stops all watching, as Discord does [cited S1].

### 5.3 Full screen in WebView2 (a trap)

WebView2 makes a full-screen element fill **only the webview**, and the host must react to
`ContainsFullScreenElementChanged` by making its own window full screen [cited S39]. Neither wry 0.57 nor tauri 2.12.1
handles it (no matches for it in their sources [cited S34]). The simplest fix stays in JS:

```text
on fullscreenchange: getCurrentWindow().setFullscreen(document.fullscreenElement !== null)
```

The Pulse window is already frameless [cited S52]. Restore the previous size and maximised state on exit. F11 is off in
WebView2 when `NewWindowRequested` is handled [cited S36], so give Pulse its own **F** key and the button. **Esc** exits.

### 5.4 Pop-out: can a second window share the stream?

- **Tauri can return a real window to `window.open`.** `WebviewWindowBuilder::on_new_window(|url, features| ...)` returns
  `NewWindowResponse::Create { window }` [cited S38]. wry then calls WebView2's `put_NewWindow`, and WebView2 returns that
  window's `WindowProxy` to the opener script [cited S37, S34]. The new webview must share the opener's environment and
  profile [cited S37]. Pulse uses a custom `data_directory` [cited S52], so build the popup with the opener's
  `window_features(features)`, which carries the opener's environment [cited S38].
- **Same origin and same process:** Chromium keeps same-origin `window.open` popups in the opener's renderer process
  [cited S40]. That makes it very likely the opener can put a `<video>` in the popup's document whose `srcObject` is the
  **same** `MediaStream`, or move the existing `<video>` element across. This is the same trick Document PiP uses [cited S41].
  Then there is **one** LiveKit connection, **one** subscription and **one** decode [speculation; spike test PO-1].
- **The alternative to avoid:** a second Tauri window that loads the app and opens its *own* `livekit-client` connection.
  That needs a third identity per user, a second PeerConnection, and a second download of the stream if the main window
  still shows it [est.]. Reject it.
- **Hygiene:** remember the pop-out's position, but clamp it on to a visible monitor on open and when displays change
  (Discord's vanishing pop-out [cited S12]).

### 5.5 Picture-in-Picture and Document PiP in WebView2

- Microsoft's list of features that differ between Edge and WebView2 does **not** list PiP as disabled [cited S36].
- I found no Microsoft statement either way, and no WebView2Feedback issue about it.
- Electron users reported `documentPictureInPicture.requestWindow()` doing nothing in an embedded Chromium [cited S41].
- Video PiP needs the embedder to provide the floating window [speculation].

**Verdict:** test `document.pictureInPictureEnabled` and `'documentPictureInPicture' in window` in the spike (PO-2). Ship the
**mini player** regardless. It covers the same need under our own control.

### 5.6 Visibility-driven quality: what `livekit-client` actually does

From the `livekit-client` 2.22.4 source [cited S30]:

- **Element size.** A shared `ResizeObserver` per attached `<video>`, debounced 100 ms, reports
  `width × height × pixelDensity` to the server.
- **Element visibility.** A shared `IntersectionObserver` with `root: null`, so it notices tiles scrolled off screen and
  tiles hidden with `display: none`. A track is paused when **no** attached element is visible; "hidden" is delayed 100 ms.
- **App background.** It listens for `document.visibilitychange` and pauses after **5 s** in background
  (`pauseVideoInBackground`, default true).
- **PiP.** It counts an element in PiP or Document PiP as visible.
- **`pixelDensity` default.** **2 if `devicePixelRatio > 2`, otherwise 1.** `'screen'` uses the real ratio.
- **Manual control.** `publication.setEnabled(false/true)` overrides adaptive visibility. `setVideoDimensions` and
  `setVideoQuality` can **cap** the request, but adaptive stream still uses a smaller size if the element is smaller.

From the LiveKit server, how a size becomes a layer [cited S31]: the server takes the requested **height**, multiplies it by
**0.9**, and picks the smallest layer whose height is at least that. If none is tall enough, it picks the top layer.

With our planned layers (fallback 720p, top 1080p) [cited S51], the layer each mode gets [est.]:

| Mode | Monitor | CSS height of video | Requested (×1, ×0.9) | Layer |
|---|---|---|---|---|
| Tile | any | ~200-270 | ~180-245 | 720p fallback |
| Focus | 1080p @100% | ~620 | 558 | 720p fallback (shown at 620 px, so 720 is already more than enough) |
| Focus | 1440p @100% | ~860 | 774 | top |
| Full screen | 1080p @100% | 1080 | 972 | top |
| Focus | 4K @150% (DPR 1.5) | ~1000 | 900 (should be 1350) | top (luckily) |
| Mini player | any | 270 | 243 | 720p fallback |

So the default `pixelDensity` under-asks by 1.25-2× on scaled displays, but with only two layers it rarely changes the
layer. Set `adaptiveStream: { pixelDensity: 'screen' }` anyway, so a future third layer doesn't make focus view on
a laptop blurry [est.].

**The WebView2/Tauri gap:**

- wry's parent window handler **skips `SIZE_MINIMIZED`** and never calls `SetIsVisible(false)` on minimise [cited S34].
- Microsoft recommends hosts do exactly that [cited S54].
- A Tauri issue confirms `document.visibilityState` "always reports itself visible", and that occlusion never changed it.
  An attempt to toggle `SetIsVisible` caused flicker and was reverted [cited S35].

So **`pauseVideoInBackground` never fires in Pulse**. A minimised viewer keeps pulling 4.5-11 Mbps and decoding
[cited S51 bitrates; est.]. Pulse needs its own small controller:

| Situation | Detect with | Action |
|---|---|---|
| Window minimised for > 5 s | Tauri `onResized` + `isMinimized()` | `setEnabled(false)` (server stops forwarding; dynacast can pause the layer if nobody else watches) |
| Restored | same | `setEnabled(true)`; first frame in ~0.25-0.6 s [cited S51] |
| Stream tile scrolled away / panel switched to a text channel while watching | IntersectionObserver (built in) | automatic pause [cited S30]. **But** switching to a text channel should keep a small "still watching" mini tile, or stop watching outright, so it doesn't silently pause |
| Pulse covered by a full-screen game on the same monitor | not reliable in WebView2 [cited S35] | none in v1; v2: Rust occlusion check [speculation] |
| User picks "Data saver" | menu | `setVideoQuality(LOW)` for that viewer only |

---

## 6. Render pipeline quality in WebView2

### 6.1 Hardware decode and overlays (zero copy)

- Chromium decodes H.264/VP9/AV1 on the GPU and hands the texture to the compositor without a CPU copy [cited S51].
- Video frame quads are **candidates for a DirectComposition overlay** (MPO): the display hardware scans the video plane
  out directly, or a video-processor blit scales it [cited S43].
- Promotion **fails** with [cited S43]:
  - backdrop filters over the video;
  - copy requests;
  - "video capture enabled";
  - complex (non-axis-aligned) transforms;
  - a non-opaque blend;
  - too many overlays (with several videos it waits 5 frames before promoting);
  - quads that moved this frame.
- When it fails, the video is composited by the GPU as an ordinary texture: still hardware, a little more GPU work and power
  [est.].

**UX rules that fall out of this:**

- No `backdrop-filter: blur()` on the controls bar over the video. Use a solid or plain translucent background, and
  auto-hide the controls after ~2 s without the mouse moving.
- No CSS `transform`, `opacity` animation or scale effect on the `<video>` itself, and no zoom-on-hover. Animate the
  container's layout, not the video.
- Keep the video rectangle still while watching. Resizing during a drag is fine, but avoid continuous small layout shifts.
- One focused stream at a time gets the best path. Extra streams in a grid will often be composited [est.].

**Whether WebView2 in windowed hosting gets MPO overlays at all is unconfirmed** [speculation]. Check in the spike that the
DevTools/GPU info reports Direct Composition with overlays, and compare GPU power in focus view with and without the controls
bar showing.

### 6.2 Frame pacing at 60 fps

- Normally Chromium shows each WebRTC frame at its scheduled render time (the jitter buffer smooths arrival). It renders
  **immediately** only when the frame is flagged low-latency or has no timestamp [cited S44].
- On a 60 Hz monitor a 60 fps stream maps 1:1.
- On 144/165 Hz monitors each frame is held for 2 or 3 refreshes, which gives slight judder. That is unavoidable for 60 fps
  content in a composited window [est.].
- `requestVideoFrameCallback` gives presented-frame timing for an fps and freeze readout in a debug overlay [est.].

### 6.3 Latency: `<video>` vs WebCodecs + canvas

- **`<video>`** adds the jitter buffer plus about 1 vsync. Most of the 80-200 ms glass-to-glass budget is capture, encode,
  network and jitter buffer, not rendering [cited S51].
- **WebCodecs + canvas/WebGPU** needs the *encoded* frames (an `RTCRtpScriptTransform`), our own jitter handling and keyframe
  logic. It loses the overlay and super-resolution paths and saves perhaps one frame [est.]. **Reject for v1.**
- **Low-latency rendering:**
  - libwebrtc sets `use_low_latency_rendering` when the stream's **playout-delay** says min = 0 and max ≤ 500 ms
    [cited S45]. Chromium then renders each frame as soon as it is decoded [cited S44].
  - The playout-delay RTP header extension exists for "certain gaming scenarios ... play the frame as soon as possible"
    [cited S46]. Zero playout delay is mainly a cloud-gaming setting [cited S46].
  - The LiveKit server can apply a playout delay room-wide (`room.playout_delay: {enabled, min, max}`) [cited S32]. In Pulse
    only screen share is video, so "room-wide" means "all streams".
  - The cost: frames arrive with network jitter, so motion looks less even [est.].
  - **Spike experiment LL-1:** `min: 0, max: 100`. Measure glass-to-glass and judder against the default. (This is not the
    "don't set playout_delay" advice in server-quality, which was about adding delay.)
- **`jitterBufferTarget`** (Chrome 124+) is only a *hint* about how much buffering you want. The browser treats it as
  guidance [cited S47]. It can't force the adaptive jitter buffer below what the network needs, so it is not a low-latency
  switch [est.]. `livekit-client`'s `setPlayoutDelay()` writes the older `playoutDelayHint` and warns if it isn't there
  [cited S30]. Treat it as legacy.

### 6.4 RTX / Intel Video Super Resolution

- Chromium switches on NVIDIA's and Intel's driver super resolution inside the **video-processor blit** of the
  overlay swap-chain path. It is off on battery, and it retries without it if the blit fails [cited S42].
- So it can only apply when the stream is presented as an overlay **and** scaled up: for example the 720p fallback layer in
  full screen on a 1440p monitor [est.].
- NVIDIA's requirements as reported:
  - RTX GPU and driver 531.18 or newer;
  - input 360p-1440p;
  - Chrome or Edge with hardware acceleration on;
  - only when the video needs upscaling [cited S48].
- Whether the driver applies it to `msedgewebview2.exe` (driver profiles target browsers) is **unknown** [speculation].
  Spike VSR-1: watch 720p in full screen on 1440p with RTX VSR on in the NVIDIA app, then look at GPU load and sharpness.
- There is no web API to request it. Treat it as a free bonus, never a design input.

---

## 7. Accessibility, shortcuts and foolproof UX

### 7.1 Avoiding the classic accidents

| Risk | Default | Why / source |
|---|---|---|
| **Sharing the wrong thing** | The picker pre-selects **Windows** (`displaySurface: 'window'`). Screens stay available, but picking one shows a one-line warning: "Everything on this screen will be visible, including notifications". After picking, the start dialog shows a **live preview** of exactly what will be sent before **Go Live** | Meet's advice [cited S23]; Chromium options [cited S26]. Whether WebView2's picker honours these options is a spike item (PK-1) [speculation] |
| **Pulse itself in the stream** (your DMs, or the "tunnel" mirror when sharing a screen) | While live, call `window.setContentProtected(true)` on every Pulse window. In tao this is `SetWindowDisplayAffinity(WDA_EXCLUDEFROMCAPTURE)`, which makes the window not appear at all in captures (Windows 10 2004+). Turn it off when the stream ends | [cited S28, S29]. Side effect: your own screenshots of Pulse are blank while you're live. Say so in a tooltip. Whether WebView2's child window inherits it needs a spike check (CP-1) [speculation] |
| **Notifications in the stream** | Window shares don't include other windows' toasts. For screen shares, Pulse mutes **its own** pop-ups and sounds while live (Discord's Streamer Mode does this [cited S6]). Windows' own Do Not Disturb has automatic rules for games, full-screen apps and duplicated displays, **not** for screen sharing [cited S27]. Chrome hides web notification content while sharing [cited S24] | We can't reliably switch Windows DND from an app [speculation], so warn instead |
| **Passwords, or "oops, wrong tab"** | A big **Pause** button next to Stop on the live bar. Viewers see "Stream paused" and the last frame blurred; snapshots stop too. Optional global hotkey | Teams and Zoom put share controls in a persistent toolbar [cited S19, S20] |
| **Forgetting you're live** | A red LIVE bar in the voice card and the window header ("You're live · 2 watching · Pause · Stop"), a taskbar overlay badge, and the system capture border where Windows draws one | Teams' red border and Zoom's green outline are their equivalents [cited S19, S20]. WGC draws a yellow border on Windows 10 (always) and optionally on 11 [cited S51] |
| **Being watched without knowing** | Viewer avatars on the live bar, and a soft sound when someone starts or stops watching (off by default for the viewer; on for the streamer) | Discord shows an eyeball per watcher in its overlay [cited S1]. LiveKit has no subscription webhook [cited S33], and the `#v` participant is hidden, so viewers report "watching X" over our gateway |
| **Stream audio surprises** | Window share: share that app's sound, on by default. Screen share: sound is off by default with a visible toggle ("Include all sound except Pulse") | Process-loopback design [cited S51]; Teams' "Include sound" toggle [cited S19] |
| **Quality jargon** | Presets named by purpose: **Smooth** (games, 720p60, the default), **Sharp** (text/code, 1080p30), **Best** (fast internet: 1080p60 or source). Numbers go in small print, and the choice is remembered | Preset numbers from [cited S51] |
| **"Why is it blurry?"** | When the encoder reports `qualityLimitationReason = bandwidth` or `cpu`, the streamer sees: "Your upload can't keep up, so friends get a lower quality". A viewer whose own download is the limit sees: "Your connection is limiting the quality" | Discord's vague "may be a network problem" frustrates users [cited S17] |
| **Accidental watch / move into voice** | Watch is always an explicit click. In a channel you're not in, the button reads "Join and watch" | [cited S13, S53] |
| **Exclusive-fullscreen games capture black** | If the first seconds arrive black or frozen, show a hint: "Set the game to **Borderless** window mode" | Discord's own advice [cited S4]; WGC limitation [cited S51] |

### 7.2 Keyboard and screen readers

- **Global:** an optional "Start/stop stream" and "Pause stream" bind in the existing keybind system [cited S52]. Unbound by
  default, like mute and deafen.
- **In the stream view:**

  | Key | Action |
  |---|---|
  | **F** | full screen |
  | **Esc** | leave full screen, or close the mini player back to normal |
  | **M** | mute stream audio |
  | **↑ / ↓** | stream volume ±10% |
  | **W** | stop watching |
  | **Tab** | moves through the controls, which appear on focus as well as on hover |

- Every control is a real `<button>` with an `aria-label` (the codebase already does this [cited S52]).
- One polite `aria-live` region announces "Alex started streaming", "You're now watching Alex" and "Alex's stream ended".
  Announcements are rate-limited.
- The LIVE pill: white on the current `--danger #d93a42` is **4.53:1**, which just passes WCAG AA for small text (4.5:1)
  [measured; cited S50]. Don't use anything lighter. `#f2616b` text on `--bg-1` is 5.42:1 [measured].
- Respect `prefers-reduced-motion`: no pulsing LIVE dot.

---

## 8. Proposed v1 UX flow

### 8.1 Streamer

**Start** (from the voice panel dock or the sidebar voice card; the screen button is beside mute and deafen):

```text
┌─ Voice panel: #general-voice ──────────────────────────────────────────────┐
│  [Alex]  [Sam]  [Jo]  [You]                                               │
│                                                                            │
│                ┌────────────────────────────────────┐                     │
│                │ [mic] [headphones] [▣ Share screen] [hang up] │          │
│                └────────────────────────────────────┘                     │
└────────────────────────────────────────────────────────────────────────────┘
```

**Pick a source** (Chromium's own picker, pre-set to Windows; spike PK-1), then the Pulse start dialog:

```text
┌─ Go live in #general-voice ───────────────────────────────── ✕ ─┐
│ ┌──────────────────────────────┐   Sharing: Rocket League       │
│ │   live preview of exactly    │   (window)  [Change]           │
│ │   what friends will see      │                                │
│ └──────────────────────────────┘                                │
│ Quality   (•) Smooth  – best for games      720p · 60 fps       │
│           ( ) Sharp   – best for text/code  1080p · 30 fps      │
│           ( ) Best    – needs fast upload   1080p · 60 fps      │
│ [✓] Share the game's sound                                      │
│ [ ] Hide preview picture from people not watching               │
│ ⓘ Pulse hides itself from your stream while you're live.        │
│                                        [Cancel]  [● Go live]    │
└─────────────────────────────────────────────────────────────────┘
```

If the user picked a whole screen, the info line becomes a warning: "Everything on this screen is visible, including
notifications. Share a window instead?"

**While live** (the sidebar card turns red; the voice panel shows your own tile):

```text
Sidebar                                   Voice panel
┌──────────────────────────────┐          ┌──────────────────────────────────────┐
│ ● LIVE  Rocket League        │          │ ┌────────────────────┐              │
│   2 watching: (S)(J)         │          │ │ your preview pic   │ ● LIVE       │
│ [⏸ Pause] [⚙] [■ Stop]       │          │ │ (what others see)  │ You · 2 👁   │
├──────────────────────────────┤          │ └────────────────────┘              │
│ Voice connected · #general   │          │ [Alex] [Sam] [Jo]                    │
└──────────────────────────────┘          └──────────────────────────────────────┘
```

- ⚙ opens: quality, change window, sound on/off, and hide preview.
- The window header also shows a small red "● LIVE" next to the connection status, so it is visible from any server tab.
- **Stop:** the card returns to normal, a toast says "You stopped streaming", and content protection is lifted.

### 8.2 Viewer

**Discover** (channel list; hover the pill for a preview):

```text
VOICE CHANNELS
🔊 general-voice                  3
    (A) Alex          [● LIVE]  ← hover:  ┌──────────────────────┐
    (S) Sam                               │ preview pic (15 s)   │
    (J) Jo                                │ Alex · Rocket League │
                                          │ 1 watching  [Watch]  │
                                          └──────────────────────┘
```

**Voice panel, not watching yet:** the streamer's tile shows the preview picture with a centred **Watch** button. If you're
not in the channel, it reads **Join and watch**.

**Watching, focus view** (the default after Watch):

```text
┌─ #general-voice ── Watching Alex ─────────────────────────── [Grid ▦] ─┐
│ ┌────────────────────────────────────────────────────────────────────┐ │
│ │                                                                    │ │
│ │                       Alex's stream (video)                        │ │
│ │                                                                    │ │
│ │ ● LIVE Alex · 720p60          (controls auto-hide after 2 s)       │ │
│ │ [🔊━━━━○ 80%] [Data saver] [⧉ Mini] [⛶ Full screen] [Stop watching] │ │
│ └────────────────────────────────────────────────────────────────────┘ │
│ (A)● (S) (J) (You)       [mic] [headphones] [▣] [hang up]              │
└────────────────────────────────────────────────────────────────────────┘
```

- **Grid** shows every stream and avatar tile at equal size.
- **Right-click** the video: stream volume, mute stream, data saver, stop watching. These are the same places Discord
  users look [cited S1, S2].
- **Mini player:** the window shrinks to an always-on-top 480×270 frame with a hover bar (volume, Expand, Stop).
- **Full screen:** F key or button. Esc leaves.

**Stream ended or interrupted:**

```text
┌──────────────────────────────────────────────┐
│   (last frame, dimmed)                       │
│   Alex stopped streaming.        [Close]     │   ← auto-closes after 5 s, returns to grid
└──────────────────────────────────────────────┘
┌──────────────────────────────────────────────┐
│   (last frame, dimmed)  ⟳ Reconnecting…      │   ← network blip: keep the last frame, don't say "ended"
└──────────────────────────────────────────────┘
```

### 8.3 Defaults table (v1)

| Setting | Default |
|---|---|
| Share target pre-selected | Window |
| Quality | Smooth (720p60) |
| Stream sound | On for windows, off for screens |
| Preview picture | Shown, refreshed every 15 s, 480×270 WebP |
| Viewer start mode | Focus, stream volume 100%, "Full" quality |
| Content protection while live | On |
| Pulse's own notifications while live | Muted |
| Pause when minimised | After 5 s |
| `adaptiveStream` | On, `pixelDensity: 'screen'` |
| Low-latency rendering | Off (pending LL-1) |

### 8.4 Not in v1

Clips, reactions, pointer and drawing, a custom picker, browser PiP, multi-stream grid tuning, and occlusion detection.

---

## 9. v2 ideas

| Idea | How, cheaply | Effort / notes |
|---|---|---|
| **Reactions over the stream** | Gateway or LiveKit data message `{emoji, x?, y?}`, floating for 2 s, rate-limited. Slack shows temporary reactions [cited S22] | Small. Keep it off the overlay path: put it beside the video, or accept compositing for 2 s |
| **Remote pointer / "look here"** | The viewer holds Alt and clicks to drop a ping at normalised (x, y), sent at ≤ 20 Hz over a data message. Everyone watching sees it. The streamer sees it on their **own** screen through a transparent, click-through, always-on-top Tauri window over the shared window | Medium. The overlay window must be content-protected so the ping isn't captured twice. Exclusive-fullscreen games will cover it [speculation] |
| **Draw on the stream** | Slack-style pencil [cited S22]: strokes as normalised polylines, fading after 5 s | Medium, the same overlay as the pointer |
| **Clips** | **Streamer side**, when Pulse owns encoding (Phase B): keep the last 30/60/120 s of the **already-encoded** H.264 in a ring buffer and mux to MP4 on a hotkey. That's nearly free because it doesn't re-encode. Discord does Alt+C, saves locally, and lets you trim and share [cited S3]. Viewer-side clipping would need `MediaRecorder` re-encoding [est.] | Medium-large; after Phase B |
| **Co-watch** (watch a video together) | A shared-state "now playing" URL plus synced play/pause and seek over the gateway. Each friend plays it locally instead of streaming it | Medium. Avoids streaming video that is already online |
| **Custom picker with thumbnails** | Once Rust does the capture: list windows with WGC thumbnails and app icons, group "Games" first, show audio meters. Chromium's picker can't be themed or pre-select a window [cited S26, S51] | Medium; part of Phase B |
| **True pop-out window** | `window.open` + `on_new_window` sharing the `MediaStream` (§5.4) | Small after PO-1 passes |
| **Occlusion-aware pause** | Rust checks whether the Pulse window is fully covered (DWM cloaked, or the foreground window is full screen on the same monitor) → `setEnabled(false)` | Small-medium [speculation] |
| **Viewer quality menu** | Auto / Full / Data saver (`setVideoQuality`) | Small |
| **"Go live" notification** | Discord's opt-in "notify friends", rate-limited [cited S5]. Only in the start dialog, never a nag [cited S9] | Small |

---

## 10. Spike additions (UX-specific; add to the existing two-day spike)

| ID | Test | Pass |
|---|---|---|
| PO-1 | Main window `window.open('about:blank')` → Tauri `on_new_window` → popup. Opener sets `popup.document.body.appendChild(video)`, or a new video whose `srcObject` is the same stream | Plays in the popup with no second connection; `getStats` shows one inbound video; CPU/GPU unchanged ±2% |
| PO-2 | `document.pictureInPictureEnabled`, `video.requestPictureInPicture()`, `'documentPictureInPicture' in window` | Record what works. Nothing depends on it |
| FS-1 | `requestFullscreen` + `setFullscreen(true)` on a frameless window, multi-monitor, Esc and F | True full screen on the window's monitor; exits cleanly; size restored |
| VIS-1 | Minimise / restore / cover with a borderless game, and log `visibilityState`, adaptive-stream visibility events and inbound bitrate | Confirms §5.6 (expected: never hidden). The custom controller drops inbound bitrate to ~0 within 6 s of minimise |
| AS-1 | Tile, focus, full screen and mini player on a 1080p@100% and a laptop @125-150% | Layer chosen matches the §5.6 table with `pixelDensity: 'screen'` |
| OV-1 | GPU info shows DirectComposition overlay support. Compare GPU power with controls shown (solid background) vs blurred background | Overlay used when controls are hidden; blur measurably costs more |
| LL-1 | LiveKit `room.playout_delay {min: 0, max: 100}` vs default; glass-to-glass (phone photo of two clocks) and visible judder | Decide whether to offer it as the default for "Smooth" |
| VSR-1 | 720p layer in full screen on 1440p with RTX VSR on | Record whether it applies in `msedgewebview2.exe` |
| PK-1 | `getDisplayMedia({video: {displaySurface: 'window'}, monitorTypeSurfaces: 'include', selfBrowserSurface: 'exclude', systemAudio: 'exclude'})` in WebView2 | Windows pane pre-selected; Pulse not listed or blank |
| CP-1 | `setContentProtected(true)` on the main window, then share the whole screen | Pulse (including its WebView2 content) is absent from the stream; the user's own screenshot shows it blank |
| SN-1 | Snapshot every 15 s during a real 1080p60 game stream (`grabFrame` + OffscreenCanvas WebP in a worker) | No measurable fps drop for the streamer; upload ≤ 32 KB at p90 |

---

## 11. Sources

All read on **2026-10-14** unless a date is given.

| ID | Source |
|---|---|
| S1 | Discord, "Go Live and Screen Share" (updated 2026-10-14). https://support.discord.com/hc/en-us/articles/360040816151 (read through the Zendesk API `/api/v2/help_center/en-us/articles/360040816151.json`) |
| S2 | Discord, "Video & Screenshare Updates - Multistream and More!" (updated 2026-09-21). https://support.discord.com/hc/en-us/articles/360045784891 |
| S3 | Discord, "Clips" (updated 2026-10-14). https://support.discord.com/hc/en-us/articles/16861982215703 |
| S4 | Discord, "[Windows] Capturing Application Window for Screen Share and Go Live" (updated 2026-10-10). https://support.discord.com/hc/en-us/articles/9410427556375 |
| S5 | Discord, "Go Live Streaming Push Notifications FAQ" (updated 2026-10-06). https://support.discord.com/hc/en-us/articles/4416120444311 |
| S6 | Discord, "Streamer Mode 101" (updated 2026-10-13). https://support.discord.com/hc/en-us/articles/218485407 |
| S7 | Discord blog, "How It All Goes Live" (2024-03-05). https://discord.com/blog/how-it-all-goes-live-an-overview-of-discords-streaming-technology |
| S8 | Discord feedback, "Customise thumbnail on streams" (2023-08-12). https://support.discord.com/hc/en-us/community/posts/16718606744983 |
| S9 | Discord feedback, "Turn off go live notification" (2019, 101 votes, comments to 2023). https://support.discord.com/hc/en-us/community/posts/360050571012 |
| S10 | Discord feedback, "picture-in-picture" (2019, 83 votes). https://support.discord.com/hc/en-us/community/posts/360043837212 |
| S11 | Discord feedback, "Enable pop-out video chat..." (2018, 5,980 votes). https://support.discord.com/hc/en-us/community/posts/360029455072 |
| S12 | Discord feedback, "Stream Pop out Missing Bug" (2021). https://support.discord.com/hc/en-us/community/posts/4409648136215 |
| S13 | Discord feedback, "Watch/Join streams without joining voice channel" (2019, 72 votes). https://support.discord.com/hc/en-us/community/posts/360055379852 |
| S14 | Discord feedback, "Client Stream Watching Quality Options" (2022). https://support.discord.com/hc/en-us/community/posts/6812957886103 |
| S15 | Discord feedback, "Multiple problems with Discord Streams" (2020). https://support.discord.com/hc/en-us/community/posts/360051383774 |
| S16 | Discord feedback, "Better Video Quality" (2019). https://support.discord.com/hc/en-us/community/posts/360050317991 |
| S17 | Discord feedback, "Discord Screen Share Is Terrible!" (2022). https://support.discord.com/hc/en-us/community/posts/4417777385623 |
| S18 | Discord feedback, "Request Control for Screenshare, and marker" (2020). https://support.discord.com/hc/en-us/community/posts/360072295571 |
| S19 | Microsoft Support, "Show your screen during a meeting" (Teams). https://prod.support.services.microsoft.com/en-us/office/show-your-screen-during-a-meeting-90c84e5a-b6fe-4ed4-9687-5923d230d3a7 (content as quoted in search results) |
| S20 | Univ. of Minnesota IT, "Zoom: Share Device Sound During Screen Share". https://it.umn.edu/services-technologies/how-tos/zoom-share-device-sound-during-screen ; Mitchell Hamline, "Optimize screen share for video clip". https://mitchellhamline.edu/technology/knowledge-base/stream-a-video-clip-in-zoom |
| S21 | Zoom Community, "System notifications are silenced" (2023). https://community.zoom.com/meetings-2/system-notifications-are-silenced-35782 |
| S22 | Slack Help, "Use huddles in Slack". https://slack.com/help/articles/4402059015315-Use-huddles-in-Slack |
| S23 | Google Workspace Help (tunnel effect when presenting). https://support.google.com/a/users/answer/13460544 |
| S24 | Google Workspace Updates, "Better privacy when screen sharing with muted web notifications" (2021-01). https://workspaceupdates.googleblog.com/2021/01/better-privacy-when-screen-sharing-chrome.html |
| S25 | Roblox DevForum, "Update on Guilded and Communities" (2025-09-29). https://devforum.roblox.com/t/update-on-guilded-and-communities/3966775 ; https://en.wikipedia.org/wiki/Guilded |
| S26 | Chrome for Developers, "Privacy-preserving screen sharing controls" (updated 2022-09-29). https://developer.chrome.com/docs/web-platform/screen-sharing-controls |
| S27 | ElevenForum, "Manage Focus Assist Automatic Rules in Windows 11". https://www.elevenforum.com/t/manage-focus-assist-automatic-rules-in-windows-11.1363/ ; Windows Central. https://windowscentral.com/how-use-focus-assist-avoid-distractions-windows-11 |
| S28 | Microsoft Learn, `SetWindowDisplayAffinity` (`WDA_EXCLUDEFROMCAPTURE`, Windows 10 2004+). https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setwindowdisplayaffinity |
| S29 | Local crate sources: tao 0.37.1 `src/platform_impl/windows/window.rs:1058` (`WDA_EXCLUDEFROMCAPTURE`); tauri 2.12.1 `src/window/mod.rs:1915` (`set_content_protected`) |
| S30 | `livekit-client` 2.22.4 (npm, published 2026-10-09) source: `room/track/RemoteVideoTrack.ts`, `Track.ts`, `RemoteTrackPublication.ts`, `RemoteTrack.ts`, `types.ts`, `options.ts` (ScreenSharePresets), `room/utils.ts`. https://www.npmjs.com/package/livekit-client |
| S31 | LiveKit server, `pkg/rtc/mediatrackreceiver.go` `GetQualityForDimension`, `layerSelectionTolerance = 0.9` (master). https://github.com/livekit/livekit/blob/master/pkg/rtc/mediatrackreceiver.go |
| S32 | LiveKit server, `config-sample.yaml` `room.playout_delay` (master). https://github.com/livekit/livekit/blob/master/config-sample.yaml |
| S33 | LiveKit docs, Webhooks (event list; no subscription event). https://docs.livekit.io/home/server/webhooks/ |
| S34 | Local crate sources: wry 0.57.0 `src/webview2/mod.rs` (L1297 `WM_SIZE if wparam != SIZE_MINIMIZED`; L769-850 `NewWindowRequested` / `SetNewWindow`; L530-575 permission mapping); no `ContainsFullScreenElement` handling in wry or tauri 2.12.1 |
| S35 | tauri-apps/tauri #10592, "document.visibilityState broken" (2024-08, open). https://github.com/tauri-apps/tauri/issues/10592 |
| S36 | Microsoft Learn, "Differences between Microsoft Edge and WebView2" (updated 2026-10-02). https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/browser-features |
| S37 | Microsoft Learn, `ICoreWebView2NewWindowRequestedEventArgs` (`put_NewWindow`; 2026-09-21). https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2newwindowrequestedeventargs |
| S38 | Local crate sources: tauri 2.12.1 `src/webview/webview_window.rs:271-320` (`on_new_window` example with `window_features`), `src/webview/mod.rs:243` (`NewWindowResponse::Create`) |
| S39 | Microsoft Learn, `CoreWebView2.ContainsFullScreenElementChanged`. https://learn.microsoft.com/en-us/dotnet/api/microsoft.web.webview2.core.corewebview2.containsfullscreenelementchanged ; ghostly PR #405 (WebView2 real full screen). https://github.com/MiguelMedeiros/ghostly/pull/405 |
| S40 | Chromium extensions group (a Chromium developer on process placement of `window.open` popups). https://groups.google.com/a/chromium.org/g/chromium-extensions/c/jAwWjEAOg4k/m/Ee7fohN7AQAJ |
| S41 | Chrome for Developers, Document Picture-in-Picture. https://developer.chrome.com/docs/web-platform/document-picture-in-picture ; electron/electron #39633. https://github.com/electron/electron/issues/39633 ; caniuse PiP. https://caniuse.com/picture-in-picture |
| S42 | Chromium `ui/gl/swap_chain_presenter.cc` (main): `ToggleNvidiaVpSuperResolution`, VP super resolution off on battery, zero-copy decode swap chain. https://chromium.googlesource.com/chromium/src/+/main/ui/gl/swap_chain_presenter.cc |
| S43 | Chromium `components/viz/service/display/dc_layer_overlay.cc` (main): `DCLayerResult` failure reasons, `is_video_frame` promotion, 5-frame delay with multiple videos. https://chromium.googlesource.com/chromium/src/+/main/components/viz/service/display/dc_layer_overlay.cc |
| S44 | Chromium `third_party/blink/renderer/modules/peerconnection/media_stream_remote_video_source.cc` (main): `render_immediately = use_low_latency_rendering || timestamp == 0`. https://chromium.googlesource.com/chromium/src/+/main/third_party/blink/renderer/modules/peerconnection/media_stream_remote_video_source.cc |
| S45 | WebRTC `modules/video_coding/timing/timing.cc` (main): `UseLowLatencyRendering()` = min playout delay 0 and max ≤ 500 ms. https://webrtc.googlesource.com/src/+/main/modules/video_coding/timing/timing.cc |
| S46 | WebRTC playout-delay header extension. https://webrtc.github.io/webrtc-org/experiments/rtp-hdrext/playout-delay/ ; BlogGeek.me, "How to Reduce WebRTC Latency". https://bloggeek.me/reducing-latency-webrtc/ |
| S47 | MDN, `RTCRtpReceiver.jitterBufferTarget`. https://developer.mozilla.org/en-US/docs/Web/API/RTCRtpReceiver/jitterBufferTarget |
| S48 | NVIDIA RTX Video FAQ. https://nvidia.custhelp.com/app/answers/detail/a_id/5448/~/rtx-video-faq ; How-To Geek, "How to Use NVIDIA Video Super Resolution". https://www.howtogeek.com/878142/how-to-use-nvidia-video-super-resolution/ (requirements as summarised in search results) |
| S49 | MDN, `ImageCapture.grabFrame()`. https://developer.mozilla.org/en-US/docs/Web/API/ImageCapture/grabFrame |
| S50 | W3C, Understanding WCAG 2.2 SC 1.4.3 Contrast (Minimum). https://www.w3.org/WAI/WCAG22/Understanding/contrast-minimum.html |
| S51 | Sibling reports (2026-10-13): `screenshare-summary.md`, `screenshare-render.md`, `screenshare-capture-encode.md`, `screenshare-server-quality.md` in this folder |
| S52 | Pulse code: `client/ui/src/routes/Shell.svelte`, `components/VoicePanel.svelte`, `components/ChannelList.svelte`, `app.css`, `lib/keybinds.ts`; `client/src-tauri/tauri.conf.json` (CSP), `client/src-tauri/src/lib.rs` (frameless window, `data_directory`); `server/src/*/routes.rs` |
| S53 | Change.org, "Let us watch streams in discord, while not being in the voice channel". https://www.change.org/p/discord-let-us-watch-streams-in-discord-while-not-being-in-the-voice-channel |
| S54 | Microsoft Learn, `CoreWebView2Controller.IsVisible` (set false when minimised). https://learn.microsoft.com/en-us/dotnet/api/microsoft.web.webview2.core.corewebview2controller.isvisible |

**M1, my measurement (2026-10-14):**

- **Images.** 17 free-game screenshots (Xonotic, SuperTuxKart, 0 A.D., Minetest, Red Eclipse, Veloren) from Wikimedia
  Commons at 1920 px wide, plus 30 desktop/app screenshots from the dev machine.
- **Chrome numbers.** Headless Google Chrome 151.0.7922.71 through playwright-core:
  `createImageBitmap` at 1920×1080 (a stand-in for `grabFrame()`) → `OffscreenCanvas` 480×270 or 640×360 → `drawImage`
  (`imageSmoothingQuality: 'high'`) → `convertToBlob` (WebP quality 0.6, JPEG 0.75). The timing covers draw plus encode, on
  software rendering on an i9-13900HX laptop.
- **libwebp numbers.** The 320×180 and desktop-screenshot rows used ImageMagick 7.1.2 with libwebp 1.6.0 at quality 60
  (centre crop to 16:9).
- **Caveats.** Real game frames vary more, especially with dense foliage or particles, so treat p90 as the planning number.
  A 1080p frame read back from the capture track is an extra GPU→CPU copy that this benchmark doesn't include (~1-3 ms
  [est.]).
- **Contrast ratios** were computed with the WCAG relative-luminance formula.

**Not verified because the web search budget ran out:** whether Discord plays "viewer joined/left" sounds (§7.1 proposes
them for Pulse without claiming Discord does); Zoom's exact "silence notifications" setting name (only community reports
were found, S21); and the date `playoutDelayHint` was removed from Chrome, if it was (S30 shows `livekit-client` still
feature-detects it).
