# Voice spike — findings (THROWAWAY code)

**Date:** 2026-10-01 · livekit 0.9.3 · livekit-server v1.13 (docker `--dev`) · Linux (CachyOS, PipeWire)

## Automated flow check — PASS
`lk room join --publish tone.ogg` (440 Hz Opus) → `voice-spike sink` received 3.7 s, **RMS 0.0876** (> 0.01), sink RSS **43 MB** (debug build).

## PlatformAudio vs manual pipeline
- `PlatformAudio` (libwebrtc ADM): mic + speakers handled by libwebrtc, browser-grade AEC/AGC/NS, device hot-swap.
  **No per-remote-participant volume API anywhere in livekit 0.9.3 / libwebrtc 0.3.50** (grep for `volume` = 0 hits).
  Remote audio is mixed and played inside libwebrtc, so we cannot scale one person.
- Manual (cpal capture → gain → APM `process_stream` → `NativeAudioSource`; remote `NativeAudioStream` per peer →
  × per-peer volume → our mixer → cpal output, mix fed to APM `process_reverse_stream`) gives **input gain and
  per-user volume** — both required by the spec (§6.3).

**Provisional decision: manual pipeline** (per-user volume is a must-have for the group). PlatformAudio stays a
reference for AEC quality comparison.

## Ear test (Greg, 2026-10-01) — manual ↔ manual on one machine, BT earbuds
- **Latency: as good as or better than Discord.** Clarity: fine ("cheap earbud", not robotic).
- In-call footprint per instance: **~28 MB PSS**, ~25% CPU (debug build; release TBD).
- `platform` ↔ `manual` produced **no audio** on this machine (no speaking events at all). PlatformAudio is
  unreliable here, so the manual pipeline is confirmed.

## BUG found + fixed: "robot voice that gets worse every rejoin"
A `NativeAudioStream` **does not end when its participant leaves**. It stays attached and receives the *next*
participant's audio too, so each rejoin adds another copy: measured 1×, 2×, 3× real-time input
(`sink`, bot joining 3×). Our 200 ms cap then discards most of it → choppy, robotic audio.
**Rule for VoiceManager:** keep a handle per remote participant and abort/drop its stream on
`TrackUnsubscribed` / `ParticipantDisconnected` (and before re-subscribing). After the fix: steady 1× rate and
tasks return to 0 after every leave.

## BT earbuds: mSBC headset codec delivers NO mic audio (environment, not Pulse)
With the earbuds in `headset-head-unit` (mSBC), the source reports RUNNING but even `parec` gets 0 samples.
`headset-head-unit-cvsd` works (real signal). Also: a profile switch mid-start strands streams opened just before
it (cpal callbacks never fire: 0 samples).
**Rules for VoiceManager:** (1) watchdog: no mic samples / no output callbacks for ~2 s → reopen the device and
tell the user; (2) ship the "Let's check" mic test early, since it would have caught this in seconds.

## Per-user volume (manual pipeline): works
Levels stable at 1× real time after the leak fix. 0.2× vs 3× was noticeable but not dramatic, because loudness is
logarithmic and >~1.5× hard-clips.
**Rule for the app:** the slider maps 0–200% onto a dB curve, with a soft limiter instead of a hard clamp.

## Pending — human ear test (Greg)
Two instances, different devices:
```
just dev-livekit
cargo run -- manual   --room t --identity a --gain 1.5 --peer-volume 1.0
cargo run -- platform --room t --identity b            # or a second `manual`
```
- [x] Both directions audible, latency feels live (better than Discord)
- [ ] Echo: speakers (not headset) on one side — does the other side hear themselves?
- [x] `m` mute/unmute works; per-user volume works (see dB note above)
- [ ] `r` RSS in-call for each mode
- [ ] Platform: `s <name>` speaker hot-swap

## Pending — Windows leg
MSVC Build Tools; `.cargo/config.toml` crt-static; same commands on the Windows partition.

## Footprint after the real client (plan 2, release build, 2026-10-01)
- Client idle + logged in: **277 MB PSS** total = pulse-app core 98 MB + WebKit web 131 MB + WebKit network 47 MB.
  The voice engine adds ~12 MB to the core; the WebKitGTK webview is the dominant cost on Linux (see spec §12 budget).
- Binary: 50 MB (libwebrtc statically linked).
- In-call numbers: measured during Greg's two-instance session.

## Linux device picking
cpal's ALSA backend lists plumbing names (`jack`, `pipewire`, `hdmi:CARD=…`), not real devices. On Linux, Pulse
follows the system default; pick devices in the OS sound settings. A PipeWire-native picker is a follow-up.

## BUG: WebKitGTK renderer abort on UI sounds (2026-10-01, Greg's first hands-on run)
Leaving voice froze user1's window: the `WebKitWebProcess` died with SIGABRT (coredump at the moment of the
"leave" sound). UI sounds were `<audio>` elements; WebKitGTK plays them through GStreamer, and without
`gst-plugins-good` (`autoaudiosink not found`) WebKit aborted instead of failing gracefully. The Rust core was
idle and healthy (main thread in its GTK loop, tokio idle, audio thread exited).
**Rule:** never play audio through the webview. UI sounds are decoded/played by the Rust core via cpal
(`client/src-tauri/src/sounds.rs`, `play_sound` command). Follow-up: reload the webview if its process dies.

## Noise suppression measurements (2026-10-06, this laptop, release build)

60 s workload: looped speech fixture + noise + a click every 150 ms. `cargo test --release --test denoise_bench -- --include-ignored --nocapture` (and `--features dfn-ll`).

| Engine | mean / p99 / max per 10 ms | % of budget | load | RSS added | delay |
|---|---|---|---|---|---|
| RNNoise (`nnnoiseless` 0.5) | 37 µs / 46 µs / 347 µs | 0.4% | — | — | 10 ms |
| DeepFilterNet3 normal (8 MB model) | 459 µs / 768 µs / 1.29 ms | 4.6% | 235 ms | 33.6 MB | 30 ms |
| DeepFilterNet3 low-latency (36 MB model) | 1.59 ms / 3.05 ms / 4.90 ms | 15.9% | 423 ms | 121 MB | 10 ms |

Windows cross-check with libDF (`cargo xwin check`): **pass** (deep_filter, tract, nnnoiseless compiled for msvc).

Gotchas:
- libDF (git `d375b2d`) needs **tract `=0.21.4`**: 0.21.7+ moved to ndarray 0.16, 0.21.6 renamed a field libDF uses. Pinned in `client/src-tauri/Cargo.toml`.
- `DfTract` is **not `Send`** (`Rc<Tensor>`, `dyn OpState`): it must be created and used on one thread → Strong runs on its own worker thread.

**Decisions:** Strong = **normal model** — low-latency saves 20 ms but costs 3.5× CPU and +87 MB RSS (rule: ≤ 1.5× CPU). Default level = **Strong** — mean 4.6% (≤ 20%) and p99 7.7% (≤ 50%) of the budget.

### Footprint after noise suppression (2026-10-06)

- Windows `pulse-app.exe` (release, `cargo xwin build --release`): **42.6 MB → 67.2 MB** (+24.6 MB: the 8 MB DFN3
  model plus tract/ONNX inference code). Linux release binary 67 MB stripped. Possible later trim: a release profile
  with LTO / `codegen-units = 1` / `strip = true` (not done: whole-app change, outside this feature).
- RAM: Strong adds ~34 MB while selected (benchmark RSS delta; freed when switching to Standard/Off). Standard/Off
  add ~nothing. In-app RSS (Off vs Strong while in voice) still to read during the ear test.
- Voice integration (`just voice-it`, Strong default): 1.00× real time on all three rejoin rounds.
