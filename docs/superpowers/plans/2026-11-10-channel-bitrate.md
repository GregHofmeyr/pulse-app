# Per-voice-channel bitrate (#75) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.
> Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** a voice channel's settings carry an audio bitrate: **Default** (today's 64 kbps) or a higher preset up to
Opus' practical max. Everyone publishing in that channel sends at it; a change reaches a call in progress live.

**Architecture:** a nullable `channels.bitrate` column (kbps; NULL = Default), surfaced as `Channel.bitrate`
(optional, skipped when absent) in everything that already carries a `Channel` (Ready, `ChannelUpdated`, listings).
`PUT /channels/{id}/bitrate` sets it (anyone in the server, voice channels only, presets only) and publishes
`ChannelUpdated`. The client passes the channel's bitrate to `join_voice` and calls `set_voice_bitrate` when a
`ChannelUpdated` changes the call's channel; the core publishes the mic with that `max_bitrate` and changes a live
mic's RTP sender parameters in place (no republish).

**Tech Stack:** Rust (axum, sqlx/SQLite, ts-rs), Tauri 2 core with LiveKit 0.9.7 (vendored), Svelte 5 + vitest.

**Status: done (2026-11-10).** Deviations: the presets end at 256 kbps, not 510 (measured below); the per-session
atomic became a per-channel one shared by the manager and that channel's calls, with the latest call's mic kept in the
manager for live changes. Deferred: #283 (music mode, no voice processing).

**Owner's decisions:** issue #75 comment (2026-11-10). Parked before: `docs/superpowers/specs/2026-10-06-noise-suppression-design.md:169-171`.

## Global Constraints

- Presets: Default (64 kbps, unchanged) + higher ones up to Opus' practical max, labelled plainly, chosen from what the
  publisher is measured to send.
- Editable by anyone in the server (flat permissions); voice channels only.
- Older clients ignore the field and keep 64 kbps; `PROTOCOL_VERSION` stays.
- Migration: next free number (0032), `ADD COLUMN` only (foreign keys OFF at migration time changes nothing).
- UI: the channel's right-click menu gets **Channel settings…** on voice channels (members only), a dialog with one
  settings field (the existing `settings-field`/`settings-label` + `Select`) and a one-line trade-off hint. Existing
  tokens only; guard tests pass.
- Anything else under "audio settings" (e.g. music mode) only if trivial, else a parked issue.

## Builder's calls (with why)

- **Presets Default (64) / 96 / 128 / 192 / 256 (max)**, from `client/src-tauri/tests/voice_bitrate_it.rs` (noise into
  the mic, the hardest input): the encoder aims at exactly the cap, but published as the app publishes (RED on) it
  sends ~260 kbps at most whatever it is allowed (384 and 510 send ~260, every run), so 384 and 510 would be labels
  with nothing behind them. With RED off, the same 510 publish sent ~510 on one connection and ~260 on the next (the
  send-side estimate above the rate both times; cause not pinned down): not a preset; #283 (music mode: RED off,
  stereo) has it. RED doubles what goes on the wire up to 192 (Default ~122 kbps, 192 ~386); from 256 its packets
  would be too big and it sends the encoder's output alone.

- **Live change via `RtpSender::set_parameters`** on the mic's sender (`encodings[0].max_bitrate`): libwebrtc's
  `WebRtcAudioSendStream::SetRtpParameters` recomputes the Opus target when `max_bitrate_bps` changes, so no
  republish (which would drop the track for a round trip and re-issue its sid). LiveKit 0.9.7 (and 0.10.0) keep the
  track's transceiver `pub(crate)`, so vendor/livekit gets a second one-line change: `LocalAudioTrack::transceiver`
  becomes `pub`. Documented beside the first in the root `Cargo.toml`.
- **Per-channel `Arc<AtomicU32>` (kbps, 0 = Default)** in the manager, shared with that channel's calls and read by
  every mic publish (join, server-mute republish) and by the SDK-republish fix-up (`LocalTrackRepublished` after a full
  reconnect re-applies it, since the SDK republishes with the options of the first publish). Rejoins keep it.
- **Separate route** rather than widening `PATCH /channels/{id}` (a rename body that older clients send must never
  reset the bitrate).

## Tasks

- [x] 1. Protocol: `Channel.bitrate: Option<u32>` (optional, skipped when None), `SetChannelBitrateRequest`,
  `CHANNEL_BITRATES_KBPS`; round-trip test (absent parses, None not serialised); regenerate TS bindings.
- [x] 2. Server: migration 0032; `CHANNEL_COLS`/`ChannelRow` read it; `PUT /channels/{id}/bitrate` with tests
  (member sets → `ChannelUpdated` + Ready carries it; reset to Default; non-voice 400; unknown value 400; non-member
  403; deleted 404).
- [x] 3. Client core: `mic_publish_options(kbps)`, live `set_mic_bitrate`, per-session atomic, `join_voice` bitrate
  arg, `set_voice_bitrate` command, `LocalTrackRepublished` fix-up; unit tests.
- [x] 4. Measurement (`tests/voice_bitrate_it.rs`, ignored, dev LiveKit on alt ports): outgoing kbps for Default and
  each preset at publish time, and after a live change.
- [x] 5. UI: `api.setChannelBitrate`, `voiceApi.join` passes the bitrate, `ChannelUpdated` for the call's channel
  → `setVoiceBitrate`; `ChannelSettingsDialog` from the channel menu; vitest.
- [x] 6. Verify (fmt, clippy, tests, xwin clippy, UI suite + check), review, rebase, merge.
