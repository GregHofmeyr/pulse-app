# Pulse

A lightweight, self-hosted voice + text chat for one friend group, and none of the bloat. Rust server (axum + SQLite),
LiveKit for voice and streams, Tauri 2 + Svelte 5 client. Windows is the main platform; Linux is the dev box.

What's in it:

- **Chat:** servers as tabs, text and voice channels in categories, DMs and groups, notes to self, replies, reactions
  and custom emoji, mentions, link embeds (YouTube, Spotify, TikTok…), GIFs (GIPHY), images and files (up to 50 MB
  kept, or up to 2 GB as a 1- or 7-day temporary send; folders zipped), search, pins, saved messages, forwards, polls,
  the picker wheel, `/` commands, and your own typing text ("cooking", "scheming"…).
- **Plans and timing:** plans with Going / Maybe / Can't and the day's forecast, Send later and your scheduled
  messages, and Remind me on any message.
- **Getting around:** the Inbox (your mentions, replies and threads from everywhere), and a quick switcher (Ctrl+K, or
  a global key) that jumps to any server, channel or chat, runs quick actions, finds builds (`builds:`) and searches
  messages everywhere you can read.
- **Voice:** calls in voice channels and DMs, push-to-talk and keybinds, soundboard, join and leave cues, a volume
  mixer, text chat beside the call, server mute and deafen, a Spotify Jam or song on the voice stage, and YouTube
  watch parties.
- **Voice processing:** noise suppression (Off, Standard, Light, Strong, or Strong (low delay), whose model downloads
  on first use), echo cancellation, a clarity EQ, even out voice volumes, a mic test that lets you hear yourself as
  others do, and a hint when your mic is clipping.
- **Streams:** Go Live (Windows) with app or screen audio in stereo, ghost viewers, and your own stream in full view.
- **Quotes, clips and builds:** a quotes channel with awards and Quote of the Year; clips channels with a games panel
  (and new NVIDIA / Game Bar recordings offered as clips); builds channels for game builds and loadouts, as a gallery
  or a list, with favourites, comments, a pinned build beside any view and a pop-out window.
- **Games and sport:** what you're playing, invites to a game, Steam achievements, Deadlock cards and stats, F1 race
  weekends and score pings for the teams you follow (each server's owner picks the channels).
- **You:** profiles and cards in your own colours, statuses (DND, Invisible, per server), "Listening to" from your
  music player (off until you turn it on), themes and accent colours, badges, two-factor sign-in, blocking,
  notifications per server and channel.
- **The app:** patch notes from the title bar, updates that install themselves (never mid-call or mid-game), and
  Report a bug (the app log and diagnostics go with it) or suggest an idea.
- **Running it:** signed releases, verified server images, encrypted backups, files and clips in Cloudflare R2
  ([`deploy/README.md`](deploy/README.md)).

There's no public download: Pulse is invite-only. The Windows installer is built by CI
([`release-windows.yml`](.github/workflows/release-windows.yml)), and after that the app updates itself from its
server.

Design: [`docs/superpowers/specs/2026-10-01-pulse-design.md`](docs/superpowers/specs/2026-10-01-pulse-design.md) ·
Voice lessons: [`spikes/voice/FINDINGS.md`](spikes/voice/FINDINGS.md) ·
Security overview and reporting a vulnerability: [`SECURITY.md`](SECURITY.md)

## Layout

| Path | What |
|---|---|
| `server/` | `pulse-server`: the axum + SQLite server (`pulse-app-server` binary), migrations in `server/migrations/` |
| `protocol/` | `pulse-protocol`: the wire types shared by both sides; ts-rs writes them to `client/ui/src/lib/protocol/` |
| `client/src-tauri/` | `pulse-client`: the Tauri 2 app's Rust core (voice, streams, clips, updater) |
| `client/ui/` | the Svelte 5 UI (Vite, vitest) |
| `deploy/` | production: compose, Caddy, LiveKit, deploy, backup and publish scripts |
| `vendor/` | patched `livekit` and `webrtc-sys` crates |
| `docs/` | how-tos; `docs/superpowers/` holds the design records and plans |

## Dev quickstart (Linux)

Needs: rustup (`rust-toolchain.toml` pins the version), clang ≥ 21, Node 22 + pnpm, Docker, `just`, WebKitGTK 4.1,
and `cargo install tauri-cli --version "^2"`.

```sh
cp .env.example .env
just dev-livekit          # LiveKit on 127.0.0.1:7880 (loopback only), webhooks → the server
just dev-s3               # optional: a fake R2 on 127.0.0.1:7895, for files and clips
just dev-server           # pulse-app-server on 127.0.0.1:7890
just invite               # prints a one-time invite code
just dev-client           # the app, hot-reloading; "Got an invite?" to register
```

`just dev-client` shows no avatars, banners or emoji on Linux. `just linux-client` builds the app like an install (a
debug build, using the system ffmpeg and GStreamer's `gst-plugins-good`) and runs it against the live server;
`just linux-run` starts that build again.

| Command | What |
|---|---|
| `just check` | fmt, clippy, all Rust tests, the generated-type check, svelte-check, vitest |
| `just gen-types` | regenerate the TypeScript protocol types after changing `protocol/` |
| `just voice-smoke` | real LiveKit webhook → server records a voice session |
| `just voice-it` | voice and stream integration tests against the dev LiveKit (real WebRTC, a fake peer, no audio devices) |
| `just voice-it-alt` | the same, with this checkout's LiveKit on 17880–17882 (beside another checkout's on 7880) |
| `just deploy-check` | validate the `deploy/` files the way CI does (needs Docker) |
| `just deny` | the dependency policy (`cargo install cargo-deny --locked`) |

CI ([`ci.yml`](.github/workflows/ci.yml)) runs the `just check` steps plus `cargo-deny`, `pnpm audit`, the deploy
checks and a server image build; the Windows-only client tests run in
[`ci-windows.yml`](.github/workflows/ci-windows.yml).

Ports: LiveKit 7880–7882 + TURN 3478/udp, server 7890, fake R2 7895, Vite 1420. Linux hotkeys:
[`docs/linux-hotkeys.md`](docs/linux-hotkeys.md). Which music players count for "Listening to":
[`docs/listening-players.md`](docs/listening-players.md).

## Licence

Pulse is MIT licensed ([LICENSE](LICENSE)). Bundled third-party components keep their own licences ([THIRD_PARTY.md](THIRD_PARTY.md)).
