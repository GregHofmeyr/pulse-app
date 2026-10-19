# Screen share deep dive: stream audio and A/V sync

Date: 2026-10-14. Research input for the screen-share spec. Nothing here is decided yet.

Scope: game sound in a stream. How we capture it on Windows, keep voices out of it, send it, play it, and keep it
in step with the video. The design being studied is: video in WebView2 (`livekit-client` as a hidden participant
`<uid>#v`), voice in Rust (LiveKit Rust SDK 0.9.3), stream audio in Rust.

Labels used on every claim:

- **[cited]**: from a source listed inline (all web sources read 2026-10-14).
- **[measured]**: I read it in source code (Pulse, the cargo registry copy of the SDK, LiveKit server, Chromium,
  libwebrtc), or ran it (the prototype tests).
- **[est.]**: my estimate from the facts above.
- **[speculation]**: an idea that needs a test before anyone relies on it.

---

## Summary

1. **Capture:** use Windows process loopback from Rust, through the `wasapi` crate 0.26. For "share a window",
   capture the game's process tree (INCLUDE). For "share a screen", capture everything except Pulse's own tree
   (EXCLUDE). Ask for 48 kHz float stereo and let Windows convert. It works on Windows 10 2004 (build 19041) and
   later in practice, although Microsoft only documents build 20348. A prototype compiles for Windows (§1.6).
2. **Chromium's `getDisplayMedia` audio cannot replace this.** Chromium's "exclude own audio" removes only its own
   audio process tree. Our voices are played by `pulse-app.exe` through cpal, which is outside that tree, so friends
   would hear themselves (§1.5).
3. **Echo:** if viewers play stream audio through our own mixer, it goes into the echo canceller's reference for
   free. Three code changes are needed first: stereo mixing, a stereo echo reference, and keying the mixer per
   *track* instead of per *person* (§2, §3.3). One echo path is not solved by this: a streamer on speakers puts the
   game into their own mic. Headphones fix it. A wilder fix is in §2.3.
4. **Transport:** the Rust SDK 0.9.3 can publish `ScreenshareAudio` with a custom bitrate, DTX off and RED off.
   It **cannot publish stereo**: it never sends the stereo flag, so LiveKit server v1.13 strips `stereo=1` and Opus
   goes out as mono, in its voice mode (§3.2, upstream issue rust-sdks #1018, still open). The fix is a small SDK
   patch, or accepting mono.
5. **A/V sync:** nothing syncs these tracks automatically. The earlier advice to "set the same `stream` id" does not
   work here, because LiveKit puts the publisher's participant ID into the sync group, and audio and video come from
   different participants. The browser's playout-timestamp stat is also empty without a synced pair (§4.1). My
   estimate is that, without any sync, audio plays 0-80 ms *after* the video. That is inside the noticeable limit
   for late audio (125 ms). So: **ship without active sync, measure in the spike, and keep a small
   measure-and-nudge controller ready** (§4.6, prototype with tests).
6. **Viewer controls:** a per-stream volume, a stream mute, stream audio only while watching, deafen covers stream
   audio, and optional ducking when someone talks (§5).

---

## 0. What the current voice code does (relevant facts)

All [measured] in `client/src-tauri/src/voice/`:

- **The mixer is mono.** Each peer queue holds mono `i16`, and `mix_into` writes the same value to every output
  channel (`mixer.rs:172`, `frame.fill(soft_limit(acc * master))`). Stereo stream audio needs a stereo bus.
- **Peers are keyed by participant identity.** `TrackSubscribed` uses `participant.identity()` as the key for the
  receive task, the mixer queue and the gain (`mod.rs:566-600`). It calls `rx.replace(id, task)`, which aborts the
  previous task for that key. `TrackUnsubscribed` removes the whole identity (`mod.rs:602`). **If a streamer
  publishes stream audio from the same Rust participant as their mic, a viewer's subscription to it would abort
  that streamer's voice task. Unsubscribing from the stream would then silence their voice.** The streamer's
  speaking ring would also light up for game sound, and their voice volume slider would also set the game's
  volume.
- **The echo reference is whatever we just played.** `on_output` downmixes the output buffer to mono and queues it
  (`devices.rs:369-415`). The mic processor feeds it to the APM before each capture block
  (`feed_echo_reference`, `devices.rs:418-434`, `process_reverse_stream(.., 1)` = mono). So anything mixed into
  our cpal output is cancelled from our mic automatically. Anything played by another process (the game, or
  WebView2) is not.
- **Bounded receive queues.** Each peer queue is capped at 200 ms (`PEER_BUFFER_MS`, `mod.rs:43`). A standing
  backlog above 40 ms + 15 ms slack is trimmed once per second (`mixer.rs:35-38`). Stream audio needs its own
  target, so that a sync delay we add on purpose is not trimmed away.
- **Remote audio is pulled on a 10 ms timer, not by the speakers.** The SDK's synthetic audio device calls
  `NeedMorePlayData` every 10 ms ([measured] `webrtc-sys-0.3.47/src/synthetic_audio_device.cpp:55-71`). So NetEq
  output arrives on the system timer and our mixer queue absorbs the drift (hence the trimming).
- **Single peer connection mode.** `Room::connect(.., RoomOptions::default())` (`mod.rs:400`), and the default is
  `single_peer_connection: true` ([measured] `livekit-0.9.3/src/room/mod.rs:475`).
- **The mic source** is `NativeAudioSource::new(AudioSourceOptions::default(), 48000, 1, 0)` (`mod.rs:412`), so
  libwebrtc's own APM is off. Our APM is the one in `devices.rs:306`.

---

## 1. Capture on Windows

### 1.1 The API

- `ActivateAudioInterfaceAsync(VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK, AUDIOCLIENT_ACTIVATION_PARAMS { type:
  PROCESS_LOOPBACK, TargetProcessId, ProcessLoopbackMode })`. The modes are INCLUDE_TARGET_PROCESS_TREE and
  EXCLUDE_TARGET_PROCESS_TREE. "The render streams of its child processes" are included or excluded with the
  target [cited:
  [MS docs, AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS](https://learn.microsoft.com/en-us/windows/win32/api/audioclientactivationparams/ns-audioclientactivationparams-audioclient_process_loopback_params)].
  There is **no "this process only" mode**.
- The capture is "not tied to a specific audio endpoint". It gets the process's audio whichever device it plays to
  [cited: [MS sample page](https://learn.microsoft.com/en-us/samples/microsoft/windows-classic-samples/applicationloopbackaudio-sample/)].
- **Minimum Windows:** the docs say Windows 10 build 20348 [cited: MS docs above]. In practice it works on
  19041 (Windows 10 2004) and later:
  - OBS documents per-application audio for "Windows 10 (Version 2004 or later) and Windows 11"
    [cited: [OBS KB](https://obsproject.com/kb/application-audio-capture-guide)];
  - win-capture-audio needs "an updated version of Windows 10 2004" [cited:
    [bozbez/win-capture-audio](https://github.com/bozbez/win-capture-audio)];
  - GStreamer lowered its check from 20348 to 19041 [cited:
    [GStreamer commit mirror](https://git.fasttube.de/FaSTTUBe/GST-Tensordecoder-ov_ep/commit/c98ad6f249f72db4eb6a2bca911f3c57808b197e)].
- **Format:** process loopback has no mix format. `GetMixFormat` is not implemented, so the caller picks one, with
  `AUDCLNT_STREAMFLAGS_LOOPBACK | EVENTCALLBACK | AUTOCONVERTPCM` in shared mode [measured: the MS sample's
  `LoopbackCapture.cpp` L95-111 asks for 44.1 kHz 16-bit stereo; cited:
  [wasapi docs](https://docs.rs/wasapi/latest/wasapi/struct.AudioClient.html), "`get_mixformat` just returns
  `Not implemented`", and the device period is ignored]. **Ask for 48 kHz float32 stereo.** Windows converts from
  whatever the game renders (5.1, 44.1 kHz, ...). We then need no resampler, and it matches `INTERNAL_RATE` and Opus.
  How good the engine's downmix and SRC are is untested [est.]. `AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY` is the
  better-quality option the sample mentions.

### 1.2 Known behaviours and bugs

| Behaviour | What to do | Label |
|---|---|---|
| A target with no audio streams gives silence ("the capturing process receives silence"). | Fine. Our loop also makes its own silence on a 10 ms clock, so the Opus stream never starves. | [cited: MS sample page] |
| The target process exits, and capture keeps returning zero frames forever with no error. | Check the PID is alive about once a second. When the game restarts with a new PID, look it up again. | [cited: [polyrec PR #143](https://github.com/yusukensanta/polyrec/pull/143)] |
| On some machines, activation succeeds but every sample is zero. | Probe by actually starting and reading. Show "no game audio detected" when the game is visibly playing but we capture silence. | [cited: [OpenWhispr PR #1935](https://github.com/OpenWhispr/openwhispr/pull/1935)] |
| Endpoint loopback skips silent periods (no packets when nothing plays). | Same answer: our own 10 ms clock. | [cited: [miniaudio #562](https://github.com/mackron/miniaudio/issues/562), [PortAudio #935](https://github.com/PortAudio/portaudio/issues/935)] |
| Exclusive-mode streams can't be loopback-captured. | Rare in games today. Tell the user if a game is silent. | [cited: [MS Loopback Recording](https://learn.microsoft.com/en-us/windows/win32/coreaudio/loopback-recording)] |
| DRM-protected streams are blocked from loopback. | Netflix-style apps will be silent (and black). That's expected. | [cited: same page] |
| Event-driven loopback didn't signal before Windows 10 1703. | Irrelevant here (process loopback needs 2004+). | [cited: same page] |
| `wasapi` 0.26 doc bug: `include_tree = false` is documented as "only the target process", but the code maps `false` to **EXCLUDE** (everything *except* the tree). | Wrap it in our own `Target::{IncludeTree, ExcludeTree}` enum (as the prototype does). | [measured: `wasapi-rs` `src/api.rs` L759-821] |
| The same naming mix-up was found in another project. | Independent confirmation that this trips people up. | [cited: [mediaway PR #99](https://github.com/nyxways/mediaway/pull/99)] |
| Are the per-app volume mixer slider or mute applied before capture? | **Unknown.** Spike: mute the game in the Windows volume mixer and see if viewers still hear it. | [speculation] |

### 1.3 From the shared window to the right process

The picker gives us a window (an HWND, or a WGC item from the JS side). Process loopback needs a PID. Chromium
already solves this for its own window-audio feature, in `GetAppMainProcessId` [measured:
`chrome/browser/media/webrtc/desktop_capture_devices_util_win.cc`, Chromium mirror
[nwjs/chromium.src](https://github.com/nwjs/chromium.src)]:

1. `GetWindowThreadProcessId(hwnd)` gives the window's PID.
2. **UWP and Store apps:** if that process is `System32\ApplicationFrameHost.exe`, find the child window with
   class `Windows.UI.Core.CoreWindow` and use *its* PID. (The frame host is not the app, and the app is not its
   child.)
3. **Everything else:** walk up the parents while they run the *same executable*, and use the oldest one. This
   catches multi-process apps like browsers and Electron games, where the window belongs to a child but the audio
   comes from a sibling.
4. Capture that PID with INCLUDE_TREE.

The prototype (§1.6) ports this to Rust. Cases to expect:

| Case | Result with the Chromium method | Label |
|---|---|---|
| A normal game (one process) | Works | [est.] |
| A game started by a launcher (Steam, EA app, Riot client) | The window process is the game exe, and its parent (the launcher) is a different exe, so we stop at the game. If the game plays audio from a *helper* process that the launcher started (not a child of the game), we miss it. | [est.] |
| Browser or Electron (audio in a utility child) | Works: the root browser process covers the audio-service child | [measured: Chromium logic; cited: [thomas-quant/wasapi-loopback](https://github.com/thomas-quant/wasapi-loopback) notes the audio service is a child] |
| UWP / Store / Game Pass titles | Works through the CoreWindow step, but not tested by us | [measured: Chromium code] |
| In-game voice chat in some games (OBS names Valorant and Call of Duty) | OBS says these need a virtual cable with app capture, which suggests the voice is rendered by another process or device path. Expect some games' voice chat to be missing in INCLUDE mode. EXCLUDE mode would pick it up. | [cited: OBS KB] |
| Anti-cheat-protected games | We never open the game's memory. We only need `PROCESS_QUERY_LIMITED_INFORMATION` (for the exe path) and the audio engine's own filtering. I found no report of process loopback being blocked by anti-cheat, but none saying it is safe either. Spike: test with the friends' games. | [speculation] |
| An elevated game (run as admin) with Pulse not elevated | `OpenProcess(QUERY_LIMITED)` normally succeeds across integrity levels. Whether loopback activation does is unknown. Discord's old advice "run Discord as admin" was for its injection-based capture. | [speculation] |

If the mapping fails, fall back to the EXCLUDE mode ("whole PC minus Pulse") and say so in the UI.

### 1.4 Which mode when

| Share | Mode | Gets | Risk |
|---|---|---|---|
| Window ("Game") | INCLUDE `app_root_pid(hwnd)` | The game only | Misses game voice chat or helper processes (§1.3) |
| Screen ("Whole PC") | EXCLUDE `std::process::id()` (Pulse) | Everything except Pulse: game, music, browser, **Windows notification sounds, other voice apps such as Discord** | Privacy: friends hear the notification "ding" and any other call. Show "Sharing all PC audio" clearly. |

- EXCLUDE with Pulse's PID also excludes Pulse's children. WebView2's `msedgewebview2.exe` processes are normally
  launched by the host app, so they would be excluded too [est.; cited weakly:
  [MS Q&A](https://learn.microsoft.com/en-us/answers/a/12737586) and forum descriptions say the parent is the host
  app]. Spike: confirm with Process Explorer. It only matters if Pulse ever plays audio from the webview (option C
  in §4.5).
- **Two streamers watching each other:** A watches B's stream. B's stream audio is played by A's Pulse, so it is
  excluded from A's own capture in both modes. No feedback loop [est.].
- Discord does the same thing for app shares: "we capture audio from the shared process and all its children"
  [cited: [Discord blog, 2024-03-05](https://discord.com/blog/how-it-all-goes-live-an-overview-of-discords-streaming-technology)].

### 1.5 Why not Chromium's `getDisplayMedia` audio

[measured from Chromium source, mirror files `chrome/browser/media/webrtc/desktop_capture_devices_util.cc` and
`media/audio/win/audio_low_latency_input_win.cc` (googlesource `main`)]:

- **Screen share + `restrictOwnAudio: true`** selects device `kLoopbackWithoutChromeId`, which runs process
  loopback in **EXCLUDE mode with `base::Process::Current().Pid()`**. That is the process running Chromium's
  audio input stream: its audio service, part of the browser's process tree. It excludes Chromium's (WebView2's)
  own audio. **It does not exclude `pulse-app.exe`**, the parent that plays all our voices through cpal. Friends
  would hear themselves. `restrictOwnAudio` shipped in Chrome 141 [cited:
  [Chrome 141 notes](https://developer.chrome.com/release-notes/141)].
- **Screen share + `suppressLocalAudioPlayback`** selects `kLoopbackWithMuteDeviceId`, which **mutes the whole
  system output** while capturing (`SetMute(true)`, L970-981). Unusable: the streamer would hear nothing.
- **Window share** (Chrome's `windowAudio: "window"`, also Chrome 141) uses the `GetAppMainProcessId` method from
  §1.3 with INCLUDE_TREE. That one *would* be fine (the game's tree doesn't contain Pulse). But this is
  Chrome-browser picker code (`chrome/browser/...`). **WebView2 has its own picker, and whether it offers window
  audio at all is unverified** [speculation]. Spike: call `getDisplayMedia({video:true, audio:true,
  windowAudio:"window"})` in WebView2 and check for an audio track on a window share.
- Earlier report, confirmed: Chromium on Windows offers system audio on whole-screen shares
  ([screenshare-render.md](screenshare-render.md) §3).

So: **capture in Rust**. Even if WebView2 window audio works, it can't do "whole PC minus Pulse", and its output
lands in the JS participant, which brings the echo trade-offs of §4.5 option C.

### 1.6 Rust crates and the prototype

- **`wasapi` 0.26.0** (2026-10-11, depends on `windows` 0.62.2):
  `AudioClient::new_application_loopback_client(pid, include_tree)`, plus a working example
  `examples/record_application.rs`. It requests 48 kHz float stereo, `EventsShared { autoconvert: true }` and an MTA
  thread, and points out that you must target the *parent* PID to get the whole tree [measured:
  github.com/HEnquist/wasapi-rs master; cited: [docs.rs](https://docs.rs/wasapi/latest/wasapi/struct.AudioClient.html)].
- **`windows` crate** directly: possible, but it means writing the `IActivateAudioInterfaceCompletionHandler` COM
  callback yourself. `wasapi` already does this. Pulse already pulls `windows` 0.62.2 into the lock file
  (cargo registry).
- **Prototype:** `/tmp/claude-1000/screenshare-audio/proto/` (scratch, not in the repo):
  - `src/win.rs`:
    - `app_root_pid(hwnd)`, the Chromium port;
    - `process_alive(pid)`;
    - `capture(Target, sink, stop)`. It turns the loopback into exact 10 ms, 48 kHz stereo `i16` frames (the
      shape `NativeAudioSource::capture_frame` wants), makes silence on a real-time clock when nothing arrives,
      and stops when an INCLUDE target dies.
  - [measured] `cargo xwin check --target x86_64-pc-windows-msvc` passes. Not run on Windows.
  - `src/sync.rs`: the sync controller from §4.6. [measured] 5 unit tests pass on Linux.
- **Track setup for publishing** (from [screenshare-capture-encode.md](screenshare-capture-encode.md), still
  valid): `NativeAudioSource::new(AudioSourceOptions { echo_cancellation: false, noise_suppression: false,
  auto_gain_control: false }, 48000, 2, 0)`, published as `TrackSource::ScreenshareAudio`. Never run stream audio
  through our APM or DeepFilterNet: those are for the mic.

---

## 2. Echo and feedback

### 2.1 Every loop we can think of

| # | Loop | Result | Fix | Label |
|---|---|---|---|---|
| E1 | The streamer's capture includes Pulse's playback (friends' voices) | Friends hear themselves, delayed | Process loopback that excludes Pulse (§1.4). Never use plain system loopback or Chromium's (§1.5). | [measured] |
| E2 | A viewer plays stream audio on speakers and their mic picks it up | Everyone hears the game twice, the second time delayed by about 2× the latency | Play stream audio **through our cpal mixer**, so it lands in the APM echo reference (`devices.rs:407`) | [measured] |
| E3 | Stereo stream audio with a mono echo reference (the current downmix) | Weaker cancellation, because left and right reach the mic through different echo paths. The residual suppressor then works harder and may clip the viewer's voice while game audio is loud. | Feed a stereo reference: `process_reverse_stream(.., 2)` is supported [measured: `libwebrtc-0.3.50/src/native/apm.rs:75`]. AEC3 detects real stereo content by default (`multi_channel.detect_stereo_content = true`) [measured: libwebrtc `api/audio/echo_canceller3_config.h` L269-274]. | [est.] |
| E4 | **The streamer on speakers:** the game plays from their speakers into their mic | Viewers hear the game twice: once clean in stream audio, once through the streamer's voice, at a different delay (a phasing or echo effect). Our APM can't cancel it, because the game isn't in our echo reference. | Headphones. Or §2.3. | [measured: the reference is only our output] |
| E5 | Pulse plays something in WebView2 (any future `<audio>`) | Not in our echo reference, so it echoes for everyone. It also leaks into the streamer's capture unless WebView2 is a child of Pulse. | Keep the "never play audio through the webview" rule | [measured] |
| E6 | A Windows "Listen to this device" setting or a virtual cable as the default output | Classic loops that we can't fix | Docs or FAQ only | [cited: [Steam Link thread](https://steamcommunity.com/app/353380/discussions/0/592889159653358597)] |

### 2.2 Viewer side: what changes in the mixer

- Add a **stream bus** next to the voice peers: stereo, with its own gain, mute, extra delay (for sync), and its
  own queue target (~20-40 ms, separate from voice's 40 ms trim) [est.].
- Mix voice (mono, centred) and stream (stereo) into a stereo frame. Then:
  - downmix only if the device is mono;
  - the playout resampler (`playout.rs`, mono today) needs to handle 2 channels;
  - the echo reference gets the stereo 48 kHz mix. (The opt-voice report's L1 already suggests feeding the
    48 kHz mix instead of the device-rate output. Do both together.)
- Give the stream its **own limiter stage** before the voice sum, and default the stream volume to about 60-70%
  [est.]. Games are mastered loud. One shared `soft_limit` at knee 0.8 would squash voices whenever an explosion
  hits.

### 2.3 The streamer's own mic (E4): options

1. **Recommend headphones** while streaming (a hint in the Go Live dialog). Zero cost.
2. **[speculation] Use the endpoint loopback as the echo reference.** Feed the APM with a standard WASAPI loopback
   of the output device, instead of (or as well as) our own mix. The loopback contains *everything* the speakers
   play (game, music, our voices) on one time base. AEC3's delay estimator absorbs the constant offset between the
   loopback and the mic. That would cancel the game from the mic for **everyone who games on speakers while in
   voice**, streaming or not. Discord-style "my friends hear my game" complaints go away.
   - Risks:
     - AEC3's residual suppressor with a loud, constant music-like reference may duck the user's own voice
       ("double-talk");
     - the two summed references would be misaligned if we keep both (use the loopback alone, since it already
       contains our output);
     - an extra WASAPI stream;
     - Bluetooth headsets that are "speakers" to Windows.
   - Test it as a separate experiment, after screen share ships.
3. A noise gate or DeepFilterNet doesn't help: game sound is not noise to them [est.].

### 2.4 Discord

- Discord's own post says nothing about echo. It does say app shares capture the process and its children
  [cited: Discord blog above].
- Public reports of "friends hear themselves" are mostly about system-audio routing (virtual cables, "Listen to
  this device", Steam streaming) rather than Go Live bugs [cited: Steam Link thread above;
  [OBS KB](https://obsproject.com/kb/application-audio-capture-guide): "set your global Desktop Audio source to
  disabled ... to avoid echo"].
- I found no authoritative Discord bug report. The thinness of the sources is a finding in itself.

---

## 3. Transport

### 3.1 Publish from Rust or from JS?

| | Rust, participant `<uid>`, `ScreenshareAudio` | JS, participant `<uid>#v`, same as the video |
|---|---|---|
| Capture | Direct (no copy) | Rust capture, then IPC (~384 KB/s for 48 kHz stereo f32, under 1% of Tauri's ~50 MB/s), then an AudioWorklet, then a `MediaStreamTrack` |
| Stereo / music Opus | **Needs an SDK patch** (§3.2) | Built in: `forceStereo`, DTX and RED off by default for stereo [measured: client-sdk-js `LocalParticipant.ts` L908-928, L1092] |
| Browser lip sync | None (different participant, §4.1) | Possible only if viewers *receive* it in JS, and the server has `room.sync_streams: true` (§4.1) |
| Viewer echo reference | Free, if received in Rust | Free only if received in Rust (which loses lip sync again) |
| Extra latency | 0 | +10-40 ms of IPC and worklet buffering [est.] |
| Moving parts | 1 | 4 (capture, IPC, worklet, JS publish), plus WebView2 audio-thread jank |

Recommendation: **publish from Rust** (as the earlier reports said), with the stereo patch.

### 3.2 Opus settings, and what Rust SDK 0.9.3 actually allows

| Setting | Wanted | Rust SDK 0.9.3 | Label |
|---|---|---|---|
| Bitrate | 96-128 kbps stereo (Opus's "sweet spot" for fullband stereo music is 64-128 kbps at 20 ms) | **Yes.** `TrackPublishOptions.audio_encoding.max_bitrate` becomes the RTP encoding's `max_bitrate`, and libwebrtc's `ComputeSendBitrate` uses `min(SDP max, rtp max)` as the encoder's **target** bitrate, not just a cap. | [measured: `local_participant.rs` L436-445; libwebrtc `media/engine/webrtc_voice_engine.cc` L239-268, L972-984]. [cited: [RFC 6716 §2.1.1](https://datatracker.ietf.org/doc/html/rfc6716)] |
| DTX | Off (DTX clips quiet game ambience) | **Yes.** `dtx: false` sends `disable_dtx`. The server records `TF_NO_DTX` and removes `usedtx=1` from its answer. | [measured: `local_participant.rs` L390; LiveKit server `pkg/rtc/participant.go` L3160, `participant_sdp.go` L388-391] |
| RED | Off (RED roughly doubles bitrate at 128k) | **Yes.** `red: false` sends `disable_red`. | [measured: L382] |
| **Stereo** | On | **No.** The SDK never sets `AddTrackRequest.stereo` or `AudioTrackFeature::TfStereo`; it only pushes `TfPreconnectBuffer`. LiveKit server (since 1.9.6, and Pulse runs v1.13.7) *removes* `;stereo=1` from the publisher answer unless the track has `TF_STEREO`. libwebrtc's Opus encoder takes its channel count from that answer, so it **encodes mono**. | [measured: `local_participant.rs` L384-399; server `participant_sdp.go` L325-397; libwebrtc `audio_encoder_opus.cc` L129-135, L253]. [cited: [rust-sdks #1018](https://github.com/livekit/rust-sdks/issues/1018), open since 2026-04-15, not fixed on `main` 2026-10-14] |
| Opus mode | `kAudio` (music) | Follows stereo: libwebrtc picks `kVoip` for 1 channel and `kAudio` for 2. Without the stereo fix, game audio is coded in voice mode. | [measured: `audio_encoder_opus.cc` L263-265] |
| In-band FEC | Doesn't matter | `useinbandfec=1` is in the default offer, so it's on. But LBRR (Opus's in-band FEC) is a SILK-layer feature, and at 96-128 kbps stereo music Opus mostly runs CELT-only, so it rarely costs or helps. | [measured: L256]. [cited: RFC 6716 §4.2.2]. [est.] for the CELT part |
| Frame size (ptime) | 20 ms (the default) | Can't be changed from the SDK. 20 ms is fine for music. | [measured: no ptime option] |

**The stereo fix:** a `[patch.crates-io]` fork of `livekit` 0.9.3 with about 5 lines:
- a `stereo: bool` on `TrackPublishOptions`;
- in `publish_track_with_video_send_encodings`, set `req.stereo = true` and push `TfStereo` when it's set.

Upstream it as the fix for #1018. The JS SDK does exactly this (`audioFeatures.push(TF_STEREO)`, `stereo:
isStereo`) [measured: client-sdk-js `LocalParticipant.ts` L1092, L1113]. Otherwise, accept mono at 96 kbps.
Friends may not notice in most games, but positional audio (footsteps left or right) is lost [est.].

**Receiving stereo in Rust:** in single-PC mode the SDK adds `stereo=1` to every Opus line in its offer
[measured: `peer_transport.rs` L339-386, L555-566], so the decoder is set up for stereo. Use
`NativeAudioStream::new(track, 48000, 2)` for stream audio. In the dual-PC fallback this munging doesn't run.
Check it if the server ever refuses single-PC [est.].

### 3.3 Receiving: required code changes

1. **Key receive tasks and mixer queues by `(identity, source)` or by track SID, not by identity** (§0). Route
   `TrackSource::ScreenshareAudio` to the stream bus.
2. **Subscribe to stream audio only while the user is watching that stream.** Tie it to the same "Watch" action
   as the video. The earlier report already plans `auto_subscribe: false` with explicit subscriptions. Otherwise a
   non-watcher hears the game, and pays ~130 kbps per stream for it.
3. Keep the leak rule: abort the receive task on unsubscribe (`rx.rs`).
4. Exclude stream audio from the speaking ring and from per-user volume.
5. The token for `<uid>` needs `can_publish_sources` to include `screen_share_audio` [est., from the render
   report's token notes].

---

## 4. A/V sync

### 4.1 Why nothing syncs these tracks for us

- **Browser lip sync needs both tracks in one sync group** (the same MSID stream) *inside one peer connection*.
  libwebrtc's `RtpStreamsSynchronizer` pairs one video receive stream with one audio receive stream
  [measured: libwebrtc `video/rtp_streams_synchronizer2.cc`]. Our audio is received by the Rust stack and our
  video by WebView2: different processes and peer connections, so there is nothing to pair.
- **Even inside one stack, LiveKit's sync group includes the publisher.** The downtrack stream ID is
  `PackSyncStreamID(publisherID, stream)`, and only if the *server* sets `room.sync_streams: true`. Otherwise it
  is `publisherID|trackID`, one group per track [measured: LiveKit server `pkg/rtc/subscribedtrack.go` L124-127,
  `participant.go` L4160-4162, `utils.go` L45-65]. `<uid>` and `<uid>#v` are different publishers, so **setting
  the same `TrackPublishOptions.stream` on both does nothing**. This corrects
  [screenshare-capture-encode.md](screenshare-capture-encode.md) §2 and
  [screenshare-server-quality.md](screenshare-server-quality.md) §4.5. (If both tracks are ever published from
  the *same* participant, the server bundles `SCREEN_SHARE` and `SCREEN_SHARE_AUDIO` into stream "screen" by
  default, but `sync_streams` must still be on.)
- **`estimatedPlayoutTimestamp` won't help.** Video sets it only in `OnSyncOffsetUpdated`, which the synchronizer
  calls only when an audio stream is paired. Audio sets it only from the synchronizer too
  [measured: libwebrtc `video/receive_statistics_proxy.cc` L417-426, L757-761; `rtp_streams_synchronizer2.cc`
  L188-204; `audio/channel_receive.cc` L1131-1146]. With no pair, the stat is empty on both sides. (MDN also lists
  Chrome as not supporting it [cited:
  [MDN](https://developer.mozilla.org/en-US/docs/Web/API/RTCInboundRtpStreamStats/estimatedPlayoutTimestamp)].)
- **The NTP clocks are close but not shared.** libwebrtc's NTP time is "monotonic clock + an offset computed
  once per process" [measured: libwebrtc `system_wrappers/source/clock.cc` L26-36]. Pulse's Rust stack and
  WebView2's renderer compute their offsets at different moments. If Windows steps the clock between those two
  moments, their NTP times differ by the step. LiveKit passes the publisher's sender-report NTP through by default
  [measured: server `pkg/sfu/rtpstats/rtpstats_sender.go` L918-946, `passThrough`]. So cross-stack NTP comparison
  is *possible*, but it's fragile and needs per-frame NTP on the audio side, which the Rust SDK doesn't expose
  (its `AudioFrame` has only data, rate and channels [measured: `libwebrtc-0.3.50/src/audio_frame.rs`]).

### 4.2 Tolerances

- **ITU-R BT.1359:** detectable at about **+45 ms (audio early) to −125 ms (audio late)**. Acceptable at about
  **+90 / −185 ms** [cited:
  [ITU-R BT.1359-0 PDF](https://itu.int/dms_pubrec/itu-r/rec/bt/R-REC-BT.1359-0-199802-S!!PDF-E.pdf)]. Watch the
  sign convention: NTIA flips it [cited: [NTIA 11-474](https://its.ntia.gov/publications/download/11-474.pdf)].
- Stricter broadcast guidance (ATSC IS-191): within 15 ms early and 45 ms late [cited:
  [TV Technology](https://www.tvtechnology.com/opinions/managing-lip-sync-265013)].
- **Early audio is about 3× more noticeable than late audio.** So aim slightly late (+10 to +20 ms audio lag), and
  when unsure, delay audio.
- Game streams are less demanding than lip sync on faces. The tell is gunshots and explosions against muzzle
  flashes [est.].

### 4.3 Expected skew without any sync

Latency budget, streamer to viewer. The network leg (streamer → SFU → viewer) is shared by both tracks and cancels
out, apart from video serialization, which is counted in the pacer line.

| Stage | Audio (Rust → Rust) | Video (WebView2 → WebView2) | Label |
|---|---|---|---|
| Capture | Loopback event period ~10 ms, so 5-10 ms average | WGC: up to 1 frame, 0-17 ms at 60 Hz | [est.] |
| Encode | Opus needs a full 20 ms frame: +20 ms | HW H.264: 3-10 ms | [est.] |
| Pacer / serialization | ~0 (small packets) | ~15 KB delta frame at 7 Mbps through the pacer: ~5-10 ms. A keyframe: 50-150 ms. | [est.] |
| Jitter buffer | NetEq: ~20-60 ms on a clean path | Target = max(min playout delay, jitter + decode + **10 ms render delay**): ~20-60 ms | [est.]; [measured: libwebrtc `modules/video_coding/timing/timing.cc` L60-68, `timing.h` L37] |
| Decode | <1 ms | HW: 2-8 ms | [est.] |
| App playout | 10 ms synthetic-device tick (avg 5) + our queue (target 20-40) + WASAPI shared output (~10-30) | Compositor and vsync: 8-30 ms | [est.]; [measured: tick and queue in §0] |
| **Total, excluding network** | **~70-165 ms** | **~45-140 ms** (keyframe: +50-150) | [est.] |

**Expected skew: audio about 0-80 ms *late*, typically ~30 ms** [est.]. That is inside the "late" limit
(−125 ms) and the right side of it.

What could break it:
- **Our mixer queue grows towards its 200 ms cap** (`PEER_BUFFER_MS`): then audio is up to ~250 ms late. Fix: a
  shorter cap for the stream bus.
- **Video stalls** (loss, NACK, keyframe): audio briefly runs *early* by 50-200 ms. That is the bad direction,
  but transient. A controller must not chase it (smoothing, §4.6).
- **NetEq stretches its buffer on a jittery Wi-Fi link:** audio later still.

### 4.4 How others do it

- **Discord:** "audio and video are sent over separate RTP packets ... the receiver synchronizes them before
  playback". Go Live is not synced with voice chat [cited: Discord blog above]. Their native engine has both
  tracks in one stack, which is the luxury we don't have.
- **Moonlight (game streaming):** no lip-sync logic at all. Both paths are made as short as possible, and audio is
  capped: "Don't queue if there's already more than 30 ms of audio data waiting" [measured: moonlight-qt
  `app/streaming/audio/renderers/sdlaud.cpp` L103-120]. Low-latency-both-paths is a legitimate sync strategy.
- **Parsec:** no public details found.

### 4.5 Options

| | What | Sync quality | Echo | Cost | Verdict |
|---|---|---|---|---|---|
| **A** | Audio from Rust, received in Rust. No active sync. Short stream bus. | ~0-80 ms audio late [est.] | Solved (§2) | Lowest | **Ship first** |
| **B** | A + a measure-and-nudge controller: delay audio in Rust, or raise video `jitterBufferTarget` in JS | Within about ±20 ms of target [est.] | Solved | +1 JS stats loop, IPC every 500 ms, mixer delay line | **Build if the spike measures \|skew\| > ~45 ms** |
| **C** | Audio captured in Rust → IPC → AudioWorklet → published by `<uid>#v` in JS. Viewers receive and play both in JS, with `sync_streams: true`. | Browser lip sync [cited mechanism] | **Broken for viewers on speakers:** WebView2's output is not in our echo reference. Patch idea: a viewer-side process loopback of the WebView2 tree, added to the reference [speculation]. | High: worklet, `setSinkId` device-following duplicated in JS, IPC jitter | Only if B fails |
| **C′** | Like C, but the viewer taps the JS audio into Rust (WebAudio → IPC → mixer) | Chrome syncs video to *its* audio sink. Our extra IPC + mixer delay is invisible to it, and adding `jitterBufferTarget` to video just makes the synchronizer delay audio too. | Solved | High | No |
| **D** | Everything in Rust (decode video in Rust) | libwebrtc lip sync within one stack (with `sync_streams`) | Solved | Software decode, frame transport (render report) | No (render report) |

### 4.6 Recommended mechanism (option B)

The idea: the network leg is shared by both tracks, so **we only need the *local* latencies at each end, and
every one of them is measurable**. Nothing depends on clocks agreeing between machines or between processes.

Signals, every 500 ms:

| Term | Where | How | Label |
|---|---|---|---|
| `audio_send_ms` | Streamer, Rust | Loopback buffering (frames waiting) + source queue + 20 ms Opus frame. Publish as a participant attribute on `<uid>` (changes slowly). | [est.] |
| `video_send_ms` | Streamer, JS | outbound-rtp: Δ`totalEncodeTime`/Δ`framesEncoded` + Δ`totalPacketSendDelay`/Δ`packetsSent` + ~1 frame of capture. Publish as an attribute on `<uid>#v`. | [est.] (the stats are standard webrtc-stats fields) |
| `audio_recv_ms` | Viewer, Rust | inbound-rtp: Δ`jitter_buffer_delay`/Δ`jitter_buffer_emitted_count` (×1000) + 5 ms (avg tick) + stream-bus queue (minus our added delay) + cpal output latency (already measured as `out_latency_ms`) | [measured: the fields exist in `libwebrtc-0.3.50/src/stats.rs` L376-382; `RemoteAudioTrack::get_stats`] |
| `video_recv_ms` | Viewer, JS | `requestVideoFrameCallback`: mean(`expectedDisplayTime` − `receiveTime`) over the frames since the last report. Send it to Rust over Tauri IPC. | [cited: [rVFC spec](https://wicg.github.io/video-rvfc/): `receiveTime` = when the last packet of the frame arrived, `expectedDisplayTime` = when it will be visible] |

Controller (prototype `src/sync.rs`, tests pass):

- `skew = (audio_send + audio_recv + added_audio_delay) − (video_send + video_recv)`, smoothed with an EWMA
  (α = 0.3) so one keyframe doesn't move anything.
- Target +15 ms (audio slightly late), deadband ±20 ms, slew ≤10 ms per update (20 ms/s).
- **Audio early:** first remove any video delay we added, then add stream-bus delay (cap 300 ms). The mixer must
  change delay smoothly: insert or drop with the existing crossfade (`backlog::crossfade`), in steps ≤10 ms.
- **Audio late:** first remove our audio delay, then raise the video receiver's `jitterBufferTarget` (cap 400 ms;
  the API accepts 0-4000) [cited:
  [MDN jitterBufferTarget](https://developer.mozilla.org/en-US/docs/Web/API/RTCRtpReceiver/jitterBufferTarget)].
  In livekit-client: `remoteVideoTrack.receiver.jitterBufferTarget = ms`. Never set it on voice.
- Hold (don't update) while the video is frozen (no rVFC callbacks for >200 ms) and for 2 s after a keyframe or
  resolution change [est.].
- Expected error: the network-leg assumption is off by about one video serialization time (a few ms at viewer
  bandwidth), plus estimation noise. ±15-25 ms in total [est.], comfortably inside BT.1359.

A cheaper cross-check if needed:
- `rVFC.captureTime` for remote frames is the sender's capture time mapped through RTCP sender reports
  [cited: rVFC spec]. Its difference from `expectedDisplayTime` gives an absolute video end-to-end latency.
- LiveKit's JS frame metadata (`frameMetadata: { timestamp: true }`, `track.lookupFrameMetadata({ rtpTimestamp })`)
  stamps every video frame with the streamer's wall clock [measured: client-sdk-js `src/frameMetadata/*`,
  `examples/demo/demo.ts` L351-373]. That would need machine clocks to agree (they often don't, by tens of ms), so
  use it only for diagnostics.

### 4.7 Measuring it (spike)

- **Automatic flash-and-beep test** [speculation, cheap to build]:
  1. The streamer opens any A/V sync test video (a flash plus a click every second) and shares that window.
  2. The viewer's JS samples a small region of each frame in the rVFC callback (draw to a 1×1 canvas) and stamps
     the flash time.
  3. Rust detects the click onset on the stream bus, at the moment it's handed to cpal plus the output latency.
  4. Skew = difference, logged per second.

  This gives the number that decides between A and B, and it later becomes a regression test.
- Fallback: record the viewer's screen and speakers with a phone at 240 fps and count frames.

---

## 5. Viewer controls

| Control | Behaviour | Notes |
|---|---|---|
| **Stream volume** | Slider on the stream tile (0-200% on the same dB curve as `percent_to_gain`), remembered per streamer | Separate from that person's voice volume (§3.3) |
| **Mute stream audio** | Unsubscribe the audio track (saves bandwidth), or just gain 0 for an instant toggle | Discord-like |
| **Only while watching** | Subscribe to audio together with the video "Watch" | §3.3 |
| **Deafen** | Mutes stream audio too (the master is 0 already: `playout_on()` → `master` in `on_output`) | Free with the bus design |
| **Ducking** (optional) | While anyone in voice is speaking (the existing per-peer speaking levels), lower the stream bus by ~8-10 dB: 50 ms attack, 500 ms release | Off by default. Some friends will hate it [est.] |
| **Sync offset** (hidden, advanced) | A ±200 ms manual offset added to the controller's target | Escape hatch while B is tuned |
| Streamer: "Share audio" toggle + mode | "Game only" (INCLUDE) / "Whole PC except Pulse" (EXCLUDE) / off | Show "no game audio detected" if we capture silence while the game is the active window |

---

## 6. Wild ideas (unranked)

1. **The endpoint loopback as the APM reference** (§2.3). It fixes "friends hear my game" for everyone on
   speakers, streaming or not. [speculation]
2. **A sync watermark:**
   - the streamer injects an inaudible marker (a ~19 kHz chirp at −50 dBFS, every 2 s) into stream audio, and
     changes one pixel's colour in the same video frame;
   - the viewer correlates the two, for exact skew with no stats at all;
   - Opus at 128 kbps keeps content up to 20 kHz.

   Fragile on the video side: scaling, chroma subsampling and encoding blur a single pixel, so use a block.
   [speculation]
3. **Audio rides in the video's data:** put compressed Opus frames into the video frames' metadata (the LiveKit
   frame metadata trailer supports `userData`) [measured: client-sdk-js "user data frame metadata trailer"
   commit, 2026-06-24]. The viewer decodes them in step with the video frame. Exact sync, but it only works from JS
   to JS, and audio then stutters whenever the video freezes. [speculation]
4. **A fallback for Windows < 2004:**
   - capture plain endpoint loopback, which includes Pulse's voices;
   - run a *second* AEC3 instance with our mixer output as the reference, to subtract our own voices digitally.
     The echo path is almost a pure delay times gain, so it should cancel well.

   Low value, since Windows 10 1909 and older are out of support. [speculation]

---

## 7. Spike checklist (adds to the summary's Day 1)

1. Process loopback, INCLUDE and EXCLUDE, on the oldest Windows 10 build a friend runs. Check activation, real
   audio, silence handling, and the game restarting.
2. `app_root_pid` on:
   - each friend's main games (launcher-started, anti-cheat);
   - one Store / Game Pass title;
   - a browser.
3. Does in-game voice chat appear in INCLUDE mode? (OBS says Valorant and Call of Duty need workarounds.)
4. The volume mixer: does muting the game in Windows mute the capture?
5. Is WebView2 a child of `pulse-app.exe` (Process Explorer)? Does WebView2's `getDisplayMedia` offer window audio
   (`windowAudio: "window"`)?
6. Publish stereo with the patched SDK. On the receiver, check that L≠R (play a left-only test tone), and that the
   encoder is in `kAudio` (getStats `codecId` and `channels`).
7. Measure the A/V skew with the flash-and-beep test: idle, under 2% loss, and on Wi-Fi. That decides between A
   and B.
8. Echo: a viewer on laptop speakers watching a loud game, with mono vs stereo reference. Do friends hear the
   game, and is the viewer's voice clipped?

---

## 8. Spec items this produces

- [ ] `voice/`: key peers by `(identity, source)`. Add a stream bus (stereo, gain, mute, delay, own queue target
  and cap ~60 ms, own limiter). Stereo playout resampler. Stereo echo reference.
- [ ] A `stream_audio` module (Windows): `wasapi` 0.26 process loopback, `Target` enum, `app_root_pid`, PID
  liveness and re-resolve, a 10 ms frame clock, publishing to a 2-channel `NativeAudioSource` as
  `ScreenshareAudio` (128 kbps, `dtx:false`, `red:false`).
- [ ] A `livekit` 0.9.3 patch adding `TrackPublishOptions.stereo` → `TfStereo` (upstream for #1018).
- [ ] Subscribe to stream audio only with "Watch". Keep it out of the speaking rings and per-user volume.
- [ ] UI: the share-audio mode, stream volume/mute, ducking toggle, and the "use headphones while streaming" hint.
- [ ] Phase 2 (if the spike shows it's needed): the sync controller (`sync.rs` prototype), JS rVFC + stats
  reporter, and streamer send-latency attributes.

---

## Sources

All read 2026-10-14.

**Microsoft**
- [AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS](https://learn.microsoft.com/en-us/windows/win32/api/audioclientactivationparams/ns-audioclientactivationparams-audioclient_process_loopback_params)
- [Application loopback sample](https://learn.microsoft.com/en-us/samples/microsoft/windows-classic-samples/applicationloopbackaudio-sample/)
  and its [source](https://github.com/microsoft/Windows-classic-samples/tree/main/Samples/ApplicationLoopback)
- [Loopback Recording](https://learn.microsoft.com/en-us/windows/win32/coreaudio/loopback-recording)
- [MS Q&A on WebView2 processes](https://learn.microsoft.com/en-us/answers/a/12737586)

**OBS and other capture projects**
- [OBS Application Audio Capture guide](https://obsproject.com/kb/application-audio-capture-guide)
- [bozbez/win-capture-audio](https://github.com/bozbez/win-capture-audio)
- [GStreamer wasapi2 19041 commit (mirror)](https://git.fasttube.de/FaSTTUBe/GST-Tensordecoder-ov_ep/commit/c98ad6f249f72db4eb6a2bca911f3c57808b197e)
- [wasapi docs.rs](https://docs.rs/wasapi/latest/wasapi/struct.AudioClient.html) and
  [HEnquist/wasapi-rs](https://github.com/HEnquist/wasapi-rs) (`src/api.rs`, `examples/record_application.rs`)
- [polyrec PR #143](https://github.com/yusukensanta/polyrec/pull/143),
  [OpenWhispr PR #1935](https://github.com/OpenWhispr/openwhispr/pull/1935),
  [miniaudio #562](https://github.com/mackron/miniaudio/issues/562),
  [PortAudio #935](https://github.com/PortAudio/portaudio/issues/935),
  [mediaway PR #99](https://github.com/nyxways/mediaway/pull/99),
  [thomas-quant/wasapi-loopback](https://github.com/thomas-quant/wasapi-loopback)

**Chromium and the web platform**
- Chromium source (googlesource `main`, and the mirror [nwjs/chromium.src](https://github.com/nwjs/chromium.src)):
  `media/audio/win/audio_low_latency_input_win.cc`, `media/audio/audio_device_description.h`,
  `chrome/browser/media/webrtc/desktop_capture_devices_util{,_win}.cc`, `services/audio/loopback_mixin.cc`
- [Chrome 141 release notes](https://developer.chrome.com/release-notes/141),
  [Intent to Prototype restrictOwnAudio](https://groups.google.com/a/chromium.org/g/blink-dev/c/ZmUiiT3rhUQ/m/d5OFE3qaAAAJ)
- [rVFC spec](https://wicg.github.io/video-rvfc/),
  [MDN jitterBufferTarget](https://developer.mozilla.org/en-US/docs/Web/API/RTCRtpReceiver/jitterBufferTarget),
  [MDN estimatedPlayoutTimestamp](https://developer.mozilla.org/en-US/docs/Web/API/RTCInboundRtpStreamStats/estimatedPlayoutTimestamp)

**libwebrtc** (webrtc.googlesource.com `main`)
- `media/engine/webrtc_voice_engine.cc`
- `modules/audio_coding/codecs/opus/audio_encoder_opus.cc`
- `video/video_receive_stream2.cc`, `video/receive_statistics_proxy.cc`, `video/rtp_streams_synchronizer2.cc`
- `audio/channel_receive.cc`
- `system_wrappers/source/clock.cc`
- `modules/video_coding/timing/timing.{cc,h}`
- `api/audio/echo_canceller3_config.h`

**LiveKit**
- Server ([livekit/livekit](https://github.com/livekit/livekit) `master`, latest release v1.13.10):
  `pkg/rtc/participant_sdp.go`, `participant.go`, `subscribedtrack.go`, `mediatrackreceiver.go`, `utils.go`,
  `pkg/sfu/downtrack.go`, `pkg/sfu/rtpstats/rtpstats_sender.go`, `CHANGELOG.md`
- [client-sdk-js](https://github.com/livekit/client-sdk-js) (v2.22.4): `LocalParticipant.ts`, `src/frameMetadata/*`,
  `examples/demo/demo.ts`
- Rust SDK (cargo registry `livekit-0.9.3`, `libwebrtc-0.3.50`, `webrtc-sys-0.3.47`,
  `livekit-protocol-0.8.0`); [rust-sdks #1018](https://github.com/livekit/rust-sdks/issues/1018)

**Others**
- [Discord: How it all goes live (2024-03-05)](https://discord.com/blog/how-it-all-goes-live-an-overview-of-discords-streaming-technology)
- [moonlight-qt](https://github.com/moonlight-stream/moonlight-qt) `sdlaud.cpp`;
  [moonlight-common-c](https://github.com/moonlight-stream/moonlight-common-c) `AudioStream.c`
- [ITU-R BT.1359-0](https://itu.int/dms_pubrec/itu-r/rec/bt/R-REC-BT.1359-0-199802-S!!PDF-E.pdf),
  [NTIA 11-474](https://its.ntia.gov/publications/download/11-474.pdf),
  [TV Technology on IS-191](https://www.tvtechnology.com/opinions/managing-lip-sync-265013)
- [RFC 6716 (Opus)](https://datatracker.ietf.org/doc/html/rfc6716)
- [Steam Link echo thread](https://steamcommunity.com/app/353380/discussions/0/592889159653358597)
