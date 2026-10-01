# Pulse

A lightweight, self-hosted voice + text chat for one friend group: servers as tabs, text and voice channels,
DMs (soon), and none of the bloat. Rust server (axum + SQLite), LiveKit for voice, Tauri 2 + Svelte 5 client.

Design: [`docs/superpowers/specs/2026-10-01-pulse-design.md`](docs/superpowers/specs/2026-10-01-pulse-design.md) ·
Voice lessons: [`spikes/voice/FINDINGS.md`](spikes/voice/FINDINGS.md)

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

Ports: LiveKit 7880–7882, server 7890, Vite 1420. Linux hotkeys: [`docs/linux-hotkeys.md`](docs/linux-hotkeys.md).
