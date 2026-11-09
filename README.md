# Pulse

A lightweight, self-hosted voice + text chat for one friend group, and none of the bloat. Rust server (axum + SQLite),
LiveKit for voice and streams, Tauri 2 + Svelte 5 client. Windows is the main platform; Linux is the dev box.

What's in it:

- **Chat:** servers as tabs, text and voice channels in categories, DMs and groups, replies, reactions and custom
  emoji, mentions, link embeds (YouTube, Spotify, TikTok…), search, pins, polls, the picker wheel, `/` commands.
- **Voice:** calls in voice channels and DMs, noise suppression, push-to-talk and keybinds, soundboard, join sounds and
  cues, a volume mixer, text chat beside the call, server mute and deafen.
- **Streams:** Go Live with app or screen audio, ghost viewers.
- **Quotes and clips:** a quotes channel with awards and Quote of the Year; clips uploaded to the clips feed.
- **You:** profiles and cards, statuses (DND, Invisible), themes and accent colours, badges, notifications per server
  and channel.
- **Running it:** self-updating Windows app, signed releases, encrypted backups ([`deploy/README.md`](deploy/README.md)).

Design: [`docs/superpowers/specs/2026-10-01-pulse-design.md`](docs/superpowers/specs/2026-10-01-pulse-design.md) ·
Voice lessons: [`spikes/voice/FINDINGS.md`](spikes/voice/FINDINGS.md)

Security overview and how to report a vulnerability: [`SECURITY.md`](SECURITY.md)

## Dev quickstart (Linux)

Needs: Rust stable, clang ≥ 21, pnpm, Docker, `just`, WebKitGTK 4.1, and `cargo install tauri-cli --version "^2"`.

```sh
cp .env.example .env
just dev-livekit          # LiveKit on 127.0.0.1:7880 (loopback only), webhooks → the server
just dev-server           # pulse-app-server on 127.0.0.1:7890
just invite               # prints a one-time invite code
just dev-client           # the app; "Got an invite?" to register
```

| Command | What |
|---|---|
| `just check` | fmt, clippy, all Rust tests, generated-type check, svelte-check, vitest (what CI runs) |
| `just voice-smoke` | real LiveKit webhook → server records a voice session |
| `just voice-it` | VoiceManager against real LiveKit; a fake peer rejoins 3× and audio must stay 1× real time |
| `just voice-it-alt` | the same, with this checkout's LiveKit on 17880–17882 (beside another checkout's on 7880) |

Ports: LiveKit 7880–7882 + TURN 3478/udp, server 7890, Vite 1420. Linux hotkeys: [`docs/linux-hotkeys.md`](docs/linux-hotkeys.md).

## Licence

Pulse is MIT licensed ([LICENSE](LICENSE)). Bundled third-party components keep their own licences ([THIRD_PARTY.md](THIRD_PARTY.md)).
