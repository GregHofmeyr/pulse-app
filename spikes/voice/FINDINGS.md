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
