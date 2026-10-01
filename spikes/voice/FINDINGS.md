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

## Pending — human ear test (Greg)
Two instances, different devices:
```
just dev-livekit
cargo run -- manual   --room t --identity a --gain 1.5 --peer-volume 1.0
cargo run -- platform --room t --identity b            # or a second `manual`
```
- [ ] Both directions audible, latency feels live
- [ ] Echo: speakers (not headset) on one side — does the other side hear themselves?
- [ ] `m` mute/unmute works; `--peer-volume 0.3` vs `2.0` clearly different
- [ ] `r` RSS in-call for each mode
- [ ] Platform: `s <name>` speaker hot-swap

## Pending — Windows leg
MSVC Build Tools; `.cargo/config.toml` crt-static; same commands on the Windows partition.
