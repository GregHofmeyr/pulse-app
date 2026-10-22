# Screen share Windows spike — Implementation Plan

> Refreshed 2026-10-22 against main a5abf01. Step zero (per-track audio) is merged; keybinds v2 (Windows low-level
> hooks) and the optimisation run are in. Paths, signatures, versions and recipes below were checked against that
> commit. Places where the refresh had to make a call are marked "(refreshed 2026-10-22: …)".

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A throwaway "spike build" of Pulse that the owner runs on Windows to measure what decides the screen-share
build:
- hardware encode;
- game-FPS cost;
- the 2-hour soak;
- stutter under loss;
- stream audio and A/V sync;
- capture of the group's games;
- pop-out handover;
- step-down signals.

**Architecture:**
- **Server:** an env-gated endpoint (`PULSE_SPIKE=1`) issues stream-room tickets shaped like the spec's (§5.2), so the
  spike exercises the real two-room layout.
- **Client, Rust (cargo feature `spike`):**
  - a log command, so every result lands in bug reports;
  - process and OS memory counters;
  - extra WebView2 browser flags from an env var;
  - a prototype stream-audio path (WASAPI process loopback published into the stream room as
    `TrackSource::ScreenshareAudio`, which step zero's `source_kind` maps to `SourceKind::Stream`);
  - beep-onset detection for the A/V sync test.
- **Client, UI:** a hidden debug panel (`Ctrl+Shift+F12`, spike builds only) built on `livekit-client`, with one
  section per test. It logs `spike:` JSON lines.

None of this merges to `main`. The **results document** and the **spec updates** are what we keep.

**Tech Stack:**
- Rust: axum 0.8 and `livekit-api` 0.8.1 (server); Tauri 2.12, `livekit` 0.9.3, `wasapi` 0.26 (new, Windows-only,
  optional) and `windows` 0.62 / `webview2-com` 0.39 (both already direct Windows dependencies) (client).
- UI: Svelte 5, `livekit-client` 2.22.4 (pinned exact; 2.23.0 shipped 2026-10-16, too new to adopt for the spike).
- Tools on the owner's PC: PresentMon (CapFrameX), clumsy, Task Manager.

**Spec:** `docs/superpowers/specs/2026-10-15-screenshare-design.md` §8.1 step 2. The test list and pass bars are in
`docs/superpowers/research/screenshare-deep-crosscheck.md` §4.

## Global Constraints

- **Branch:** `screenshare-spike` off `main`. Never merged; deleted after the results are written up (with the owner's
  OK).
- **Every result** is a single `spike:` log line of JSON (`{"test":"S1",...}`) so a bug report carries it. Reports
  keep 256 KB of log tail, so the owner files a report after each test section.
- **Prod changes need the owner's OK each time:** deploying the spike server image, `PULSE_SPIKE=1`, and LiveKit
  codec-config switches. Revert all of them when the spike ends.
  - (refreshed 2026-10-22: how the spike image gets built.) `release-server.yml` only builds `main` automatically.
    Dispatching it by hand on `screenshare-spike` works, but its `publish` job tags the image `:<sha7>` **and
    `:latest`**, so `:latest` would point at spike code until the next `main` build. Deploy by sha
    (`just deploy <spike-sha7>`, run from the spike worktree so its `deploy/compose.yml` is the one copied), and at
    the end `just deploy <main-sha7>` from `main` and re-run `release-server.yml` on `main` to put `:latest` back.
  - `just deploy` copies the repo's `deploy/livekit.yaml` to prod, which silently undoes `just spike-codecs on`. Deploy
    first, switch codecs after.
- **Hardware:** the owner's PC (RTX 2080 Ti, no iGPU) as the streamer. A second PC as the viewer (the owner's
  partner's 4070 Ti Super). Swap roles once for S1/S2 on the 4070 Ti Super.
- **Games for capture and FPS tests:** Overwatch, Apex Legends, CS2.
- **Out of scope:**
  - S11 (server load on a scratch box): the 2 OCPU resize happens anyway.
  - S16 (Rust pass-through viewer).
  - S17 (Linux viewer).
  - S14 and S15 run only if time is left.
- **Spike builds keep the normal app working:** voice and chat as usual. The panel is additive.
- Commit messages end with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

## Review Focus

1. **A spike ticket must never grant the voice room.** Even a throwaway endpoint on prod mustn't open an eavesdropping
   hole. Pinned in Task 1 (`spike_tickets_only_name_the_stream_room`).
2. **The endpoint is off unless `PULSE_SPIKE=1`.** A forgotten deploy must not leave it live. Pinned in Task 1
   (`spike_tickets_are_404_when_disabled`).
3. **A failed test must still log a line,** for example getDisplayMedia cancelled, or the encoder not reported yet.
   Silence looks like "not run". Pinned in Task 5: every runner wraps in `logResult`, which logs `ok:false` with the
   error.
4. **Loopback capture of a process that exits mid-test** (a game restart in S9) must end with a logged error, not hang
   the panel. Pinned in Task 4 (`capture_ends_cleanly_when_the_process_exits`).
5. **The soak sampler mustn't itself leak** (S3 measures leaks). It holds no growing arrays: only the latest sample
   and a fixed 120-entry ring. Pinned in Task 5 (`ring_never_grows_past_its_capacity`).

---

## File Structure

| File | Purpose |
|---|---|
| `server/src/spike.rs` | `POST /spike/stream/{channel}/ticket` (env-gated) |
| `server/src/config.rs`, `server/src/lib.rs`, `server/src/testing.rs` | The `spike` flag; mount the router; `test_config()` |
| `deploy/livekit.codecs.yaml` | The `room.enabled_codecs` block for S1(b) |
| `deploy/compose.yml` | Pass `PULSE_SPIKE` through to `pulse-server` |
| `justfile` | `spike-codecs on|off`, `spike-server on|off` |
| `client/src-tauri/Cargo.toml` | The `spike` feature: optional `wasapi`; extra features on the existing `windows` dep |
| `client/src-tauri/src/spike/mod.rs` | Commands: `spike_log`, `spike_ticket`, `spike_os_stats`, `spike_window_pid` |
| `client/src-tauri/src/spike/loopback.rs` | Process loopback capture to `NativeAudioSource`; the `#a` room connection |
| `client/src-tauri/src/spike/onset.rs` | Beep-onset detector (pure, tested) |
| `client/src-tauri/src/lib.rs` | Extra WebView2 args; renderer-crash hook; register spike commands under `cfg(feature = "spike")` |
| `client/src-tauri/src/api.rs` | `Api::spike_ticket` |
| `client/src-tauri/src/voice/mod.rs` | `VoiceManager::spike_mixer()` (spike-only accessor to step zero's `MixerInput`) |
| `client/src-tauri/tauri.spike.conf.json` | Spike-only config overlay: CSP for LiveKit, the pop-out window's capability |
| `client/ui/src/spike/*.ts`, `client/ui/src/spike/SpikePanel.svelte` | The debug panel and its pure helpers (tested with vitest) |
| `.github/workflows/release-windows.yml` | A `spike` input that builds with the feature, the config overlay and `VITE_SPIKE=1` |
| `docs/superpowers/research/screenshare-spike-protocol.md` | The owner's step-by-step script |
| `docs/superpowers/research/screenshare-spike-results.md` | Results, written after the runs |

---

### Task 1: Server spike tickets (env-gated)

**Files:**
- Create: `server/src/spike.rs`
- Modify: `server/src/config.rs` (add `pub spike: bool` to `Config`; read it in `Config::from_env`, `config.rs:25-44`)
- Modify: `server/src/lib.rs` (`pub mod spike;` next to the other modules, lines 1-14; `.merge(spike::router())` in
  `router()`, lines 25-38, after `.merge(reports::routes::router())` and before `.fallback(...)`)
- Modify: `server/src/testing.rs` (`test_config()`, lines 48-60, sets `spike: false`)
- Test: `server/tests/spike.rs`

**Interfaces:**
- Produces: `POST /spike/stream/{channel}/ticket` with body `{"role":"v"|"v2"|"a"}`, returning
  `VoiceTokenResponse { url, token }`.
  - Room: `<channel>~s`.
  - Identity: `<uid>#<role>`.
  - Grants: `room_join`; `can_subscribe: true`; `can_publish: true` with `can_publish_sources: ["screen_share"]` for
    `v`, `[]` (subscribe-only) for `v2`, and `["screen_share_audio"]` for `a`.
  - Requires: server membership (the same `is_server_member` check `voice/routes.rs`'s `token` makes), and the caller
    in that channel's voice (`s.voice.members(id).contains(&me)`; `VoiceState` has no `is_in`, and `members` already
    answers it).
  - Returns 404 when `spike` is false.

- [ ] **Step 1: Write the failing tests**

`server/tests/spike.rs` (`testing::post_json` takes the body as a `serde_json::Value` by value; `TestApp` exposes
`voice: VoiceState` directly, so the test marks a user as in voice with `app.voice.join(..)`: no helper needed):

```rust
use livekit_api::access_token::TokenVerifier;
use pulse_server::testing;
use serde_json::json;

async fn lounge(app: &testing::TestApp, token: &str) -> String {
    let s = testing::create_server(app, token, "Main").await;
    let chans: Vec<pulse_protocol::rest::Channel> =
        testing::get_json(app, token, &format!("/servers/{}/channels", s.id)).await;
    chans
        .into_iter()
        .find(|c| c.name.as_deref() == Some("Lounge"))
        .unwrap()
        .id
        .to_string()
}

#[tokio::test]
async fn spike_tickets_are_404_when_disabled() {
    let app = testing::spawn().await; // spike: false
    let (_, tok) = testing::register(&app, "alex").await;
    let ch = lounge(&app, &tok).await;
    let res = testing::post_json(&app, &tok, &format!("/spike/stream/{ch}/ticket"), json!({"role":"v"})).await;
    assert_eq!(res.status(), 404);
}

#[tokio::test]
async fn spike_tickets_only_name_the_stream_room() {
    let mut cfg = testing::test_config();
    cfg.spike = true;
    let app = testing::spawn_with(cfg.clone()).await;
    let (uid, tok) = testing::register(&app, "alex").await;
    let ch = lounge(&app, &tok).await;
    app.voice.join(ch.parse().unwrap(), uid, "PA_test"); // what the LiveKit join webhook does
    for (role, sources) in [("v", vec!["screen_share"]), ("a", vec!["screen_share_audio"]), ("v2", vec![])] {
        let res = testing::post_json(&app, &tok, &format!("/spike/stream/{ch}/ticket"), json!({"role":role})).await;
        assert_eq!(res.status(), 200, "{role}");
        let body: pulse_protocol::rest::VoiceTokenResponse = res.json().await.unwrap();
        let claims = TokenVerifier::with_api_key(&cfg.livekit_key, &cfg.livekit_secret)
            .verify(&body.token)
            .unwrap();
        assert_eq!(claims.video.room, format!("{ch}~s"), "never the voice room");
        assert_eq!(claims.sub, format!("{uid}#{role}"));
        assert_eq!(claims.video.can_publish_sources, sources, "{role}");
    }
}

#[tokio::test]
async fn spike_tickets_need_you_in_voice() {
    let mut cfg = testing::test_config();
    cfg.spike = true;
    let app = testing::spawn_with(cfg).await;
    let (_, tok) = testing::register(&app, "alex").await;
    let ch = lounge(&app, &tok).await;
    let res = testing::post_json(&app, &tok, &format!("/spike/stream/{ch}/ticket"), json!({"role":"v"})).await;
    assert_eq!(res.status(), 403);
}
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p pulse-server --test spike`
Expected: compile errors (no `spike` field on `Config`); once that exists, 404s for the enabled cases (no route yet).

- [ ] **Step 3: Implement**

`server/src/config.rs`: add `pub spike: bool` to `Config` (with a doc comment: "THROWAWAY: the screen-share spike's
ticket endpoint, `PULSE_SPIKE=1`"), read in `from_env` as
`spike: std::env::var("PULSE_SPIKE").is_ok_and(|v| v == "1")`.

`server/src/testing.rs`: in `test_config()` set `spike: false`. Nothing else: `TestApp` already has
`pub voice: crate::voice::VoiceState` (and a `state()` method), and `VoiceState::members(channel)` is the "who is in
this channel" query, so neither a test helper nor a new `is_in` is needed. (refreshed 2026-10-22: the old
`mark_in_voice` helper and `is_in` method are dropped as redundant.)

`server/src/spike.rs`:

```rust
//! THROWAWAY (screen-share spike). Stream-room tickets shaped like the spec's §5.2, behind PULSE_SPIKE=1.

use axum::extract::{Path, State};
use axum::routing::post;
use axum::{Json, Router};
use livekit_api::access_token::{AccessToken, VideoGrants};
use pulse_protocol::ids::ChannelId;
use pulse_protocol::rest::VoiceTokenResponse;
use serde::Deserialize;

use crate::AppState;
use crate::access::channel_for;
use crate::auth::AuthUser;
use crate::error::{AppError, AppResult};

pub fn router() -> Router<AppState> {
    Router::new().route("/spike/stream/{id}/ticket", post(ticket))
}

#[derive(Deserialize)]
struct Req {
    role: String,
}

async fn ticket(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<ChannelId>,
    Json(req): Json<Req>,
) -> AppResult<Json<VoiceTokenResponse>> {
    if !s.cfg.spike {
        return Err(AppError::NotFound);
    }
    let ch = channel_for(&s.db, me, id).await?;
    // same rule as voice/routes.rs `token`: server voice needs server membership
    if let Some(server) = ch.server_id
        && !crate::servers::routes::is_server_member(&s.db, server, me).await?
    {
        return Err(AppError::Forbidden);
    }
    if !s.voice.members(id).contains(&me) {
        return Err(AppError::Forbidden);
    }
    let sources: Vec<String> = match req.role.as_str() {
        "v" => vec!["screen_share".into()],
        "a" => vec!["screen_share_audio".into()],
        "v2" => vec![],
        _ => return Err(AppError::BadRequest("role is v, v2 or a".into())),
    };
    let jwt = AccessToken::with_api_key(&s.cfg.livekit_key, &s.cfg.livekit_secret)
        .with_identity(&format!("{me}#{}", req.role))
        .with_ttl(std::time::Duration::from_secs(10 * 60))
        .with_grants(VideoGrants {
            room_join: true,
            room: format!("{id}~s"),
            can_publish: Some(!sources.is_empty()),
            can_publish_sources: sources,
            can_subscribe: Some(true),
            can_publish_data: Some(false),
            ..Default::default()
        })
        .to_jwt()
        .map_err(|e| AppError::Internal(e.into()))?;
    tracing::info!(user = %me, channel = %id, role = %req.role, "spike ticket");
    Ok(Json(VoiceTokenResponse { url: s.cfg.livekit_url.clone(), token: jwt }))
}
```

`AppError::NotFound` (404), `Forbidden` (403), `BadRequest(String)` and `Internal(anyhow::Error)` all exist in
`error.rs`; no change there. In `lib.rs` add `pub mod spike;` and `.merge(spike::router())` (before `.fallback`).
The handler uses `let`-chains, as `voice/routes.rs` does (edition 2024).

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p pulse-server --test spike` and then `cargo test -p pulse-server`
Expected: PASS. The webhook (`voice/routes.rs` `webhook`) already ignores `~s` rooms: it acks and returns when
`room.name.parse::<ChannelId>()` or `identity.parse::<UserId>()` fails, and `"<id>~s"` / `"<uid>#a"` fail both. Add
this assertion to `spike_tickets_only_name_the_stream_room` to pin it:
`assert!(format!("{ch}~s").parse::<pulse_protocol::ids::ChannelId>().is_err());`

- [ ] **Step 5: Commit**

```bash
git add server/src/spike.rs server/src/config.rs server/src/lib.rs server/src/testing.rs server/tests/spike.rs
git commit -m "spike(server): env-gated stream-room tickets (never the voice room)

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 2: LiveKit codec switch and deploy recipes

**Files:**
- Create: `deploy/livekit.codecs.yaml`
- Modify: `justfile` (append the recipes)
- Modify: `deploy/compose.yml`: pass `PULSE_SPIKE` through to `pulse-server` (add `PULSE_SPIKE: ${PULSE_SPIKE:-0}` to
  its `environment:` map, after `RUST_LOG`)

**Interfaces:**
- Produces:
  - `just spike-codecs on|off`: appends the block to prod's `livekit.yaml` and restarts LiveKit, or restores the repo
    copy;
  - `just spike-server on|off`: sets `PULSE_SPIKE` in prod `.env` and recreates `pulse-server`.

- [ ] **Step 1: Write the codec block**

`deploy/livekit.codecs.yaml`. This is LiveKit v1.13.10 `pkg/config/config.go`'s full default `room.enabled_codecs`
list (checked 2026-10-22: PCMU, PCMA, opus, red, VP8, H264, VP9, AV1, H265, rtx; `CodecSpec` keys `mime` and
`fmtp_line`), plus strict Main and Baseline H.264. Never trim it: dropping `audio/red` or `video/rtx` silently breaks
voice loss recovery and video retransmission. Prod's `deploy/livekit.yaml` has no `room:` section today, so appending
one doesn't create a duplicate key; re-check that before switching.

```yaml
# SPIKE S1(b): appended to livekit.yaml on prod by `just spike-codecs on`.
room:
  enabled_codecs:
    - mime: audio/PCMU
    - mime: audio/PCMA
    - mime: audio/opus
    - mime: audio/red
    - mime: video/VP8
    - mime: video/H264
    - mime: video/H264
      fmtp_line: "level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=4d001f"
    - mime: video/H264
      fmtp_line: "level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42001f"
    - mime: video/VP9
    - mime: video/AV1
    - mime: video/H265
    - mime: video/rtx
```

Validate it with LiveKit itself (the image prod runs, `deploy/compose.yml`), so the YAML keys are checked rather than
guessed. Concatenate exactly what prod will get:
`cat deploy/livekit.yaml deploy/livekit.codecs.yaml > /tmp/lk-spike.yaml`. Then run
`docker run --rm --network host -e LIVEKIT_KEYS="pulse: secretsecretsecretsecretsecretsecret" -v /tmp/lk-spike.yaml:/c.yaml livekit/livekit-server:v1.13.10 --config /c.yaml`
and check that the log shows no config error before it starts listening. Stop it with Ctrl+C. The key must be named
`pulse`: `livekit.yaml`'s `webhook.api_key` is `pulse`, and LiveKit refuses to start if that key isn't in
`LIVEKIT_KEYS`. (refreshed 2026-10-22: was `k:`.) Stop the dev LiveKit (`docker compose down`) first if it's running:
both bind 7880 on the host. If LiveKit rejects `fmtp_line`, check its `config.go` `CodecSpec` field names for v1.13.10
and fix the key.

- [ ] **Step 2: Add the recipes**

Append to `justfile`, after the production recipes (they use its existing `prod := "pulse@pulse"` variable):

```make
# SPIKE: switch prod LiveKit to the strict-H.264 codec list (S1b) and back. Needs the owner's OK.
spike-codecs mode:
    #!/usr/bin/env bash
    set -euo pipefail
    if [ "{{mode}}" = on ]; then
      cat deploy/livekit.yaml deploy/livekit.codecs.yaml > /tmp/livekit.spike.yaml
      scp /tmp/livekit.spike.yaml {{prod}}:/srv/pulse/livekit.yaml
    else
      scp deploy/livekit.yaml {{prod}}:/srv/pulse/livekit.yaml
    fi
    ssh {{prod}} 'cd /srv/pulse && docker compose restart livekit && sleep 3 && docker compose logs --tail 5 livekit'

# SPIKE: enable/disable the spike ticket endpoint on prod. Needs the owner's OK.
spike-server mode:
    ssh {{prod}} 'cd /srv/pulse && sed -i "/^PULSE_SPIKE=/d" .env && echo "PULSE_SPIKE={{ if mode == "on" { "1" } else { "0" } }}" >> .env && docker compose up -d pulse-server'
```

- [ ] **Step 3: Check**

Run: `just deploy-check` (validates `deploy/compose.yml` against `deploy/env.example`, the Caddyfile, and
shellchecks `deploy/*.sh`; it does not lint justfile recipes) and `just --list` (parses the justfile, so the new
recipes show up).
Expected: PASS, and `spike-codecs` / `spike-server` listed.

- [ ] **Step 4: Commit**

```bash
git add deploy/livekit.codecs.yaml deploy/compose.yml justfile
git commit -m "spike(deploy): codec-list switch and spike endpoint toggles for prod

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 3: Client spike feature: log, tickets, OS counters, WebView2 flags, crash hook

**Files:**
- Modify: `client/src-tauri/Cargo.toml`
- Create: `client/src-tauri/src/spike/mod.rs`
- Modify: `client/src-tauri/src/lib.rs` (`create_main_window`, lines 26-50; the `invoke_handler` list, lines 146-189)
- Modify: `client/src-tauri/src/api.rs` (add `spike_ticket` after `voice_token`, line 480)
- Test: `client/src-tauri/src/spike/mod.rs` (`mod tests`)

**Interfaces:**
- Produces Tauri commands (spike builds only):
  - `spike_log(line: String)`: logs `spike: <line>` at info with target `spike`.
  - `spike_ticket(channel_id: ChannelId, role: String) -> VoiceTokenResponse`
  - `spike_os_stats() -> OsStats`, where `OsStats` is
    `{ pulse_private_mb: f64, webview_private_mb: f64, handles: u32, nonpaged_pool_mb: f64, dwm_ws_mb: f64, audiodg_ws_mb: f64, gpu_dedicated_mb: Option<f64> }`
    (camelCase over IPC).
  - `spike_window_pid(title: String) -> Option<u32>`
- An env var `PULSE_WEBVIEW2_ARGS` is appended to the WebView2 browser args (Windows).
- On a WebView2 `ProcessFailed`, log `spike: {"test":"S12","event":"renderer_failed",...}`.

- [ ] **Step 1: Cargo feature and deps**

(refreshed 2026-10-22: `windows` 0.62 is now a normal, non-optional Windows dependency, used by `voice/rtprio.rs` and
the keybind hooks in `hotkeys.rs`, and `webview2-com` 0.39 / `windows-core` 0.62 are direct Windows dependencies for
`webview_memory.rs`. So the spike can't make `windows` optional. Instead, it turns on the two extra `windows`
features it needs, and only `wasapi` is optional.)

In `client/src-tauri/Cargo.toml`, the existing `[features]` table becomes:

```toml
[features]
# Strong uses DeepFilterNet3's low-latency model (36 MB) instead of the normal one (8 MB).
dfn-ll = ["deep_filter/default-model-ll"]
# THROWAWAY: the screen-share spike build (debug panel commands, loopback prototype).
spike = [
  "dep:wasapi",
  "windows/Win32_System_ProcessStatus",
  "windows/Win32_System_Diagnostics_ToolHelp",
]
```

and add one line to the existing `[target.'cfg(windows)'.dependencies]` table (don't add a second table, and leave
its `windows` entry and its feature list as they are):

```toml
# THROWAWAY (spike): WASAPI process loopback. 0.26.0 depends on windows/windows-core ^0.62.2, the versions in Cargo.lock.
wasapi = { version = "0.26", optional = true }
```

The `windows` dependency already enables `Win32_Foundation`, `Win32_System_Threading` and
`Win32_UI_WindowsAndMessaging`, which the spike code also uses. Check with `cargo tree -p pulse-client --features
spike --target x86_64-pc-windows-msvc -i windows` that there's still one `windows` (0.62.2) in the tree.

- [ ] **Step 2: Write the failing test (the pure part)**

`spike/mod.rs` keeps the parsing of `PULSE_WEBVIEW2_ARGS` pure:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extra_args_are_appended_to_the_defaults() {
        assert_eq!(
            webview_args("--disable-features=A", Some("--enable-features=B  --x")),
            "--disable-features=A --enable-features=B --x"
        );
        assert_eq!(webview_args("--disable-features=A", None), "--disable-features=A");
        assert_eq!(webview_args("--disable-features=A", Some("   ")), "--disable-features=A");
    }
}
```

Run: `cargo test -p pulse-client --lib --features spike spike::`
Expected: FAIL (`webview_args` not found). (`tauri::generate_context!` needs `client/ui/dist` to exist; if it doesn't,
`mkdir -p client/ui/dist`, as CI does.)

- [ ] **Step 3: Implement**

`client/src-tauri/src/spike/mod.rs`:

```rust
//! THROWAWAY (screen-share spike): commands for the hidden debug panel. Results are `spike:` log lines,
//! so every bug report carries them.

pub mod loopback; // builds on every OS; only its WASAPI capture is Windows-only (Task 4)
pub mod onset;

use pulse_protocol::ids::ChannelId;
use pulse_protocol::rest::VoiceTokenResponse;
use serde::Serialize;
use tauri::State;

use crate::api::ApiError;
use crate::commands::Core;

/// wry's default WebView2 args plus whatever the spike run asks for (S1 HW off, S6 throttling, S7 decode).
pub fn webview_args(defaults: &str, extra: Option<&str>) -> String {
    let extra: Vec<&str> = extra.unwrap_or("").split_whitespace().collect();
    std::iter::once(defaults).chain(extra).collect::<Vec<_>>().join(" ")
}

#[tauri::command]
pub fn spike_log(line: String) {
    tracing::info!(target: "spike", "spike: {line}");
}

#[tauri::command]
pub async fn spike_ticket(
    core: State<'_, Core>,
    channel_id: ChannelId,
    role: String,
) -> Result<VoiceTokenResponse, ApiError> {
    let (api, token) = core.current()?; // what join_voice uses: (Api, session token) or Unauthorized
    api.spike_ticket(&token, channel_id, &role).await
}

#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct OsStats {
    pulse_private_mb: f64,
    webview_private_mb: f64,
    handles: u32,
    nonpaged_pool_mb: f64,
    dwm_ws_mb: f64,
    audiodg_ws_mb: f64,
    gpu_dedicated_mb: Option<f64>,
}

#[tauri::command]
pub fn spike_os_stats() -> OsStats {
    #[cfg(windows)]
    {
        win::os_stats()
    }
    #[cfg(not(windows))]
    {
        OsStats::default()
    }
}

#[tauri::command]
pub fn spike_window_pid(title: String) -> Option<u32> {
    #[cfg(windows)]
    {
        win::window_pid(&title)
    }
    #[cfg(not(windows))]
    {
        let _ = title;
        None
    }
}

#[cfg(windows)]
mod win {
    use super::OsStats;
    use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM};
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
    };
    use windows::Win32::System::ProcessStatus::{
        GetPerformanceInfo, GetProcessMemoryInfo, PERFORMANCE_INFORMATION, PROCESS_MEMORY_COUNTERS_EX,
    };
    use windows::Win32::System::Threading::{
        GetCurrentProcessId, GetProcessHandleCount, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows::Win32::UI::WindowsAndMessaging::{EnumWindows, GetWindowTextW, GetWindowThreadProcessId};
    use windows::core::BOOL;

    const MB: f64 = 1024.0 * 1024.0;

    /// (pid, parent pid, exe name) for every process.
    fn processes() -> Vec<(u32, u32, String)> {
        let mut out = Vec::new();
        unsafe {
            let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return out };
            let mut e = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
            if Process32FirstW(snap, &mut e).is_ok() {
                loop {
                    let n = e.szExeFile.iter().position(|c| *c == 0).unwrap_or(e.szExeFile.len());
                    out.push((e.th32ProcessID, e.th32ParentProcessID, String::from_utf16_lossy(&e.szExeFile[..n])));
                    if Process32NextW(snap, &mut e).is_err() {
                        break;
                    }
                }
            }
            let _ = CloseHandle(snap);
        }
        out
    }

    /// (private bytes, working set) of one process, in MB.
    fn mem(pid: u32) -> (f64, f64) {
        unsafe {
            let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { return (0.0, 0.0) };
            let mut c = PROCESS_MEMORY_COUNTERS_EX::default();
            let ok = GetProcessMemoryInfo(h, &mut c as *mut _ as *mut _, std::mem::size_of_val(&c) as u32).is_ok();
            let _ = CloseHandle(h);
            if ok { (c.PrivateUsage as f64 / MB, c.WorkingSetSize as f64 / MB) } else { (0.0, 0.0) }
        }
    }

    pub fn os_stats() -> OsStats {
        let me = unsafe { GetCurrentProcessId() };
        let procs = processes();
        // WebView2's browser process is our child; its renderers/GPU process are ITS children.
        let mut tree = vec![me];
        let mut i = 0;
        while i < tree.len() {
            let parent = tree[i];
            tree.extend(procs.iter().filter(|p| p.1 == parent).map(|p| p.0));
            i += 1;
        }
        let webview: f64 = tree[1..].iter().map(|pid| mem(*pid).0).sum();
        let by_name = |n: &str| procs.iter().find(|p| p.2.eq_ignore_ascii_case(n)).map_or(0.0, |p| mem(p.0).1);
        let mut handles = 0u32;
        unsafe {
            let _ = GetProcessHandleCount(windows::Win32::System::Threading::GetCurrentProcess(), &mut handles);
        }
        let mut perf = PERFORMANCE_INFORMATION::default();
        unsafe {
            let _ = GetPerformanceInfo(&mut perf, std::mem::size_of_val(&perf) as u32);
        }
        OsStats {
            pulse_private_mb: mem(me).0,
            webview_private_mb: webview,
            handles,
            nonpaged_pool_mb: perf.KernelNonpaged as f64 * perf.PageSize as f64 / MB,
            dwm_ws_mb: by_name("dwm.exe"),
            audiodg_ws_mb: by_name("audiodg.exe"),
            // DXGI per-adapter usage needs IDXGIAdapter3::QueryVideoMemoryInfo; the protocol has the owner read
            // Task Manager's "Dedicated GPU memory" instead (S3), so None here.
            gpu_dedicated_mb: None,
        }
    }

    pub fn window_pid(title: &str) -> Option<u32> {
        struct Ctx<'a> { want: &'a str, found: Option<u32> }
        unsafe extern "system" fn each(hwnd: HWND, lp: LPARAM) -> BOOL {
            let ctx = unsafe { &mut *(lp.0 as *mut Ctx) };
            let mut buf = [0u16; 512];
            let n = unsafe { GetWindowTextW(hwnd, &mut buf) } as usize;
            if n > 0 && String::from_utf16_lossy(&buf[..n]) == ctx.want {
                let mut pid = 0u32;
                unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
                ctx.found = Some(pid);
                return BOOL(0);
            }
            BOOL(1)
        }
        let mut ctx = Ctx { want: title, found: None };
        unsafe {
            let _ = EnumWindows(Some(each), LPARAM(&mut ctx as *mut Ctx as isize));
        }
        ctx.found
    }
}
```

These paths and signatures were checked against `windows` 0.62.2 on 2026-10-22: `BOOL` is `windows::core::BOOL`;
`OpenProcess`, `CreateToolhelp32Snapshot` return `Result<HANDLE>`; `Process32FirstW/NextW`, `GetProcessMemoryInfo`,
`GetPerformanceInfo`, `GetProcessHandleCount`, `EnumWindows` return `Result<()>`; `GetWindowTextW` takes
`&mut [u16]`; `GetWindowThreadProcessId` takes `Option<*mut u32>`. Windows CI (Task 6) is still the real check: fix
any path it rejects without changing the logic.

In `api.rs`, add after `voice_token` (`ChannelId` and `VoiceTokenResponse` are already imported; `Self::parse` is the
same private helper `voice_token` uses):

```rust
    #[cfg(feature = "spike")]
    pub async fn spike_ticket(&self, token: &str, channel: ChannelId, role: &str) -> Result<VoiceTokenResponse, ApiError> {
        Self::parse(
            self.http
                .post(format!("{}/spike/stream/{channel}/ticket", self.base))
                .bearer_auth(token)
                .json(&serde_json::json!({ "role": role }))
                .send()
                .await,
        )
        .await
    }
```

There is no `api_and_token()`. Every command, `join_voice` included, gets the `Api` and the session token with
`let (api, token) = core.current()?;` (`Core::current`, `commands.rs:51`, `pub(crate)`, synchronous, returns
`ApiError::Unauthorized` when signed out). `spike_ticket` above uses that.

In `lib.rs`:
- add `#[cfg(feature = "spike")] pub mod spike;` with the other `pub mod` lines (1-17).
- **WebView2 args:** in `create_main_window`. Today it is one chain from `tauri::WebviewWindowBuilder::new(app, "main",
  ...)` to `.data_directory(profile_dir.join("webview")).build()?` (lines 27-34). Split it so the spike can add one
  call before `build()`:

```rust
    let b = tauri::WebviewWindowBuilder::new(app, "main", tauri::WebviewUrl::default())
        // ...the existing .title(..) .. .data_directory(..) calls, unchanged...
        ;
    #[cfg(all(windows, feature = "spike"))]
    let b = b.additional_browser_args(&spike::webview_args(
        // wry 0.57's default (webview2/mod.rs): keep it, or these features come back on
        "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection",
        std::env::var("PULSE_WEBVIEW2_ARGS").ok().as_deref(),
    ));
    let win = b.build()?;
```

  Tauri's docs warn that webviews with different browser args need different data directories. The main window
  already has its own per-profile one, so that holds. Don't run a spike build and a normal build on the same profile
  at the same time. `autoplay` isn't set, so wry would add no other default args.
- **Renderer crash (S12), Windows:** after the window is built. It goes next to the existing Linux crash block and
  `webview_memory::install(&win)` (lines 35-47), using the same `with_webview` pattern as
  `webview_memory::set_target`:

```rust
    #[cfg(all(windows, feature = "spike"))]
    win.with_webview(|wv| {
        use webview2_com::Microsoft::Web::WebView2::Win32::COREWEBVIEW2_PROCESS_FAILED_KIND;
        use webview2_com::ProcessFailedEventHandler;
        let Ok(core) = (unsafe { wv.controller().CoreWebView2() }) else {
            tracing::warn!(target: "spike", "spike: no CoreWebView2 for the ProcessFailed hook");
            return;
        };
        let mut token = 0i64;
        let _ = unsafe {
            core.add_ProcessFailed(
                &ProcessFailedEventHandler::create(Box::new(|_, args| {
                    let mut kind = COREWEBVIEW2_PROCESS_FAILED_KIND::default();
                    let kind = match args {
                        Some(a) if unsafe { a.ProcessFailedKind(&mut kind) }.is_ok() => kind.0,
                        _ => -1,
                    };
                    tracing::error!(target: "spike", "spike: {{\"test\":\"S12\",\"event\":\"renderer_failed\",\"kind\":{kind}}}");
                    Ok(())
                })),
                &mut token,
            )
        };
    })?;
```

  (refreshed 2026-10-22: in `webview2-com-sys` 0.39.1 `ProcessFailedKind` takes an out-pointer
  (`*mut COREWEBVIEW2_PROCESS_FAILED_KIND`) and `add_ProcessFailed` takes `*mut i64`. The snippet follows wry's
  own `add_PermissionRequested` usage.) `webview2-com` 0.39 is already a direct Windows dependency (for
  `webview_memory.rs`), so the feature needs nothing more for it.
- **Register the commands:** `tauri::generate_handler!` can't take `cfg` attributes on individual entries, and
  today `run()` is one chain, `builder.setup(..).on_window_event(..).invoke_handler(generate_handler![..]).build(..)
  .expect(..).run(..)`. Split it after `.on_window_event(..)` into `let builder = ...;`, then:
  - `#[cfg(feature = "spike")] let builder = builder.invoke_handler(tauri::generate_handler![...all existing..., spike::spike_log, spike::spike_ticket, spike::spike_os_stats, spike::spike_window_pid, spike::loopback::spike_loopback_start, spike::loopback::spike_loopback_stop, spike::loopback::spike_beep_watch]);`
  - `#[cfg(not(feature = "spike"))] let builder = builder.invoke_handler(tauri::generate_handler![...])` with the
    original list (42 entries today, `commands::login` through `reports::save_report`);
  - then `builder.build(tauri::generate_context!()).expect(..).run(..)` as now.

  The loopback commands come in Task 4; add them to the list then. They build on every OS (Task 4 keeps only the
  WASAPI capture under `cfg(windows)`), so one spike list works on Linux and Windows.
  (refreshed 2026-10-22: the old plan put `mod loopback` under `cfg(windows)` but still listed its commands, which
  wouldn't compile in the Linux spike build that Step 4 runs.)

- [ ] **Step 4: Run tests and both builds**

Run: `cargo test -p pulse-client --lib --features spike spike::` → PASS.
Run: `cargo build -p pulse-client` and `cargo build -p pulse-client --features spike` (on Linux, the Windows-only code
is cfg'd out) → both build.
Run: `cargo clippy -p pulse-client --all-targets --features spike -- -D warnings`. CI's clippy runs without the
feature, so this is the only lint of the spike code.

- [ ] **Step 5: Commit**

```bash
git add client/src-tauri Cargo.lock  # wasapi lands in the workspace lockfile at the repo root
git commit -m "spike(client): log/ticket/OS-stats commands, WebView2 args, renderer-crash hook

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 4: Stream-audio prototype: process loopback, the `#a` connection, beep onset

**Files:**
- Create: `client/src-tauri/src/spike/onset.rs` (pure, tested on any OS)
- Create: `client/src-tauri/src/spike/loopback.rs` (builds on every OS; the WASAPI capture inside is `cfg(windows)`)
- Modify: `client/src-tauri/src/voice/mod.rs` (one spike-only accessor on `VoiceManager`, below)
- Modify: `client/src-tauri/src/lib.rs` (`app.manage(spike::loopback::Loopback::default())` in `setup`)
- Test: `client/src-tauri/src/spike/onset.rs`; `client/src-tauri/src/spike/loopback.rs` (Windows-only test)

**Interfaces:**
- Consumes:
  - Task 3's `spike_ticket`;
  - step zero (merged in a5abf01), in `voice/mixer.rs`:
    - `MixerInput::add_source(&self, generation: u64, source: &str, owner: &str, kind: SourceKind)`;
    - `push_in(&self, generation: u64, source: &str, samples: &[i16])` (mono, 48 kHz i16);
    - `remove_source(&self, generation: u64, source: &str)`;
    - `generation()`;
    - sources keyed by track SID; gain per (owner, `SourceKind`).
  - step zero's `voice/mod.rs` `source_kind(TrackSource::ScreenshareAudio) == SourceKind::Stream`: the prototype
    publishes with `TrackSource::ScreenshareAudio`, so a later voice-side receive path classifies it the way the real
    build will.
  - New, spike-only, in `voice/mod.rs` next to the other diagnostics accessors (`mixer_sources` etc., ~line 1675):

    ```rust
    /// THROWAWAY (spike): the current call's mixer input, so the spike's `#a` connection plays through it.
    #[cfg(feature = "spike")]
    pub async fn spike_mixer(&self) -> Option<Arc<mixer::MixerInput>> {
        self.session.lock().await.as_ref().map(|s| s.pipeline.shared.mix_in.clone())
    }
    ```

    `Shared::mix_in` is already `pub mix_in: Arc<MixerInput>` (`voice/devices.rs`). This is the only spike change
    inside `voice/`. (refreshed 2026-10-22: added because step zero's mixer has no outside handle.)
- Produces Tauri commands:
  - `spike_loopback_start(channel_id: ChannelId, mode: "include" | "exclude", pid: Option<u32>, tone: Option<String>)`
    returns `Result<(), String>`. `tone` is `"left"` to publish a left-only 1 kHz tone instead of loopback (stereo
    check).
  - `spike_loopback_stop()`
  - `spike_beep_watch(channel_id: ChannelId)`: joins `~s` as `#a`, subscribe-only. It plays received stream audio and
    logs `spike: {"test":"S9","event":"beep","t_ms":<unix ms>}` for each detected beep onset.
- Pure: `onset::Detector::new(rate: u32)`, `.feed(&[f32]) -> Vec<usize>` (sample offsets of onsets in this chunk).

- [ ] **Step 1: Write the failing onset tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn tone(n: usize, amp: f32) -> Vec<f32> {
        (0..n).map(|i| amp * (i as f32 * 2.0 * std::f32::consts::PI * 1000.0 / 48_000.0).sin()).collect()
    }

    #[test]
    fn finds_a_beep_after_silence_at_the_right_sample() {
        let mut d = Detector::new(48_000);
        let mut x = vec![0.0; 4800];
        x.extend(tone(2400, 0.5));
        let hits = d.feed(&x);
        assert_eq!(hits.len(), 1);
        assert!((hits[0] as i64 - 4800).abs() <= 48, "within 1 ms: {}", hits[0]);
    }

    #[test]
    fn a_long_tone_is_one_onset_and_quiet_noise_is_none() {
        let mut d = Detector::new(48_000);
        assert!(d.feed(&vec![0.001; 48_000]).is_empty());
        assert_eq!(d.feed(&tone(48_000, 0.5)).len(), 1);
    }

    #[test]
    fn onsets_across_chunk_boundaries_count_once() {
        let mut d = Detector::new(48_000);
        let mut x = vec![0.0; 4800];
        x.extend(tone(4800, 0.5));
        let (a, b) = x.split_at(4900);
        assert_eq!(d.feed(a).len() + d.feed(b).len(), 1);
    }
}
```

Run: `cargo test -p pulse-client --lib --features spike spike::onset` → FAIL (no `Detector`).

- [ ] **Step 2: Implement the detector**

```rust
//! THROWAWAY (S9 A/V sync): find beep onsets in received stream audio. 1 ms RMS windows; an onset is
//! a window above 0.1 after at least 200 ms below 0.02 (the test pattern beeps 100 ms every 2 s).

pub struct Detector {
    win: usize,
    buf: Vec<f32>,
    quiet_windows: usize,
    need_quiet: usize,
    consumed: usize,
}

impl Detector {
    pub fn new(rate: u32) -> Self {
        let win = (rate / 1000) as usize;
        Self { win, buf: Vec::with_capacity(win), quiet_windows: usize::MAX / 2, need_quiet: 200, consumed: 0 }
    }

    /// Feed samples; returns the offsets (into THIS chunk) where a beep starts.
    pub fn feed(&mut self, x: &[f32]) -> Vec<usize> {
        let mut out = Vec::new();
        for (i, v) in x.iter().enumerate() {
            self.buf.push(*v);
            if self.buf.len() == self.win {
                let rms = (self.buf.iter().map(|s| s * s).sum::<f32>() / self.win as f32).sqrt();
                self.buf.clear();
                if rms > 0.1 {
                    if self.quiet_windows >= self.need_quiet {
                        out.push(i + 1 - self.win);
                    }
                    self.quiet_windows = 0;
                } else if rms < 0.02 {
                    self.quiet_windows = self.quiet_windows.saturating_add(1);
                }
            }
        }
        self.consumed += x.len();
        out
    }
}
```

Run: `cargo test -p pulse-client --lib --features spike spike::onset` → PASS.

- [ ] **Step 3: Implement loopback publish and listen**

`client/src-tauri/src/spike/loopback.rs` outline. The code goes in as written below. The `wasapi` 0.26.0 calls were
checked on 2026-10-22: `initialize_mta() -> HRESULT`;
`AudioClient::new_application_loopback_client(process_id: u32, include_tree: bool)`;
`initialize_client(&mut self, &WaveFormat, &Direction, &StreamMode)`;
`StreamMode::EventsShared { autoconvert, buffer_duration_hns }`;
`WaveFormat::new(storebits, validbits, &SampleType, rate, channels, Option<u32>)`;
`set_get_eventhandle`, `get_audiocaptureclient`, `read_from_device_to_deque(&mut VecDeque<u8>)`,
`wait_for_event(u32)`. If one has changed, the reference is the crate's `examples/record_application.rs` (there is
no `loopback_application.rs`). Note that example's advice: for a game that runs as several processes, target the
parent PID.

(refreshed 2026-10-22: the whole module builds on every OS. Only `wasapi_capture` (the WASAPI half of
`capture_loop`) and `process_alive` are `#[cfg(windows)]`. Elsewhere a stub `wasapi_capture` returns
`Err("process loopback is Windows-only")`.
The left-tone mode and `spike_beep_watch` work on Linux too, and Task 3's single spike handler list compiles there.)

```rust
//! THROWAWAY (S9, S19): stream audio via WASAPI process loopback, published as `<uid>#a` into `<ch>~s`.

use std::sync::{Arc, Mutex};
use livekit::options::TrackPublishOptions;
use livekit::prelude::*;
use livekit::webrtc::audio_source::native::NativeAudioSource;
use livekit::webrtc::audio_source::{AudioSourceOptions, RtcAudioSource};
use livekit::webrtc::audio_frame::AudioFrame; // the path voice/processor.rs uses
use pulse_protocol::ids::ChannelId;
use tauri::State;

use crate::commands::Core;

/// The publishing side (`spike_loopback_start`) and the listening side (`spike_beep_watch`): each keeps its
/// `Room` here, or dropping it would disconnect.
#[derive(Default)]
pub struct Loopback(Mutex<Option<Running>>, Mutex<Option<Watching>>);
struct Running { room: Room, stop: Arc<std::sync::atomic::AtomicBool>, thread: Option<std::thread::JoinHandle<()>> }
struct Watching { room: Room, tasks: Vec<tokio::task::JoinHandle<()>> }

#[tauri::command]
pub async fn spike_loopback_start(
    core: State<'_, Core>, lb: State<'_, Loopback>,
    channel_id: ChannelId, mode: String, pid: Option<u32>, tone: Option<String>,
) -> Result<(), String> {
    let t = super::spike_ticket(core, channel_id, "a".into()).await.map_err(|e| e.to_string())?;
    let (room, _ev) = Room::connect(&t.url, &t.token, RoomOptions::default()).await.map_err(|e| e.to_string())?;
    // stereo 48 kHz (needs the rust-sdks #1018 patch to really go out as stereo; S9 checks L≠R on the far end).
    // webrtc-sys is already patched from vendor/webrtc-sys (the log-sink change, root Cargo.toml [patch.crates-io]):
    // if #1018 touches webrtc-sys, apply it to that vendored copy rather than adding a second patch.
    // A 100 ms queue, unlike the mic's queue 0 (voice/mod.rs): WASAPI hands over bursts.
    let src = NativeAudioSource::new(AudioSourceOptions::default(), 48_000, 2, 100);
    let track = LocalAudioTrack::create_audio_track("stream-audio", RtcAudioSource::Native(src.clone()));
    room.local_participant()
        // ScreenshareAudio: step zero's `source_kind` maps it to SourceKind::Stream (its own volume, no speaking ring)
        .publish_track(LocalTrack::Audio(track), TrackPublishOptions {
            source: TrackSource::ScreenshareAudio, dtx: false, red: false, ..Default::default()
        })
        .await.map_err(|e| e.to_string())?;
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let (stop2, rt) = (stop.clone(), tokio::runtime::Handle::current());
    let target = match (mode.as_str(), pid) {
        ("include", Some(p)) => (p, true),
        // EXCLUDE our own tree: Pulse + its WebView2 children never reach the stream
        _ => (std::process::id(), false),
    };
    let thread = std::thread::spawn(move || {
        let r = capture_loop(target.0, target.1, tone.as_deref(), &stop2, |chunk: Vec<i16>| {
            let frame = AudioFrame {
                samples_per_channel: (chunk.len() / 2) as u32,
                data: chunk.into(), sample_rate: 48_000, num_channels: 2,
            };
            let _ = rt.block_on(src.capture_frame(&frame));
        });
        let line = match r {
            Ok(()) => r#"{"test":"S9","event":"capture_ended","ok":true}"#.to_string(),
            Err(e) => format!(r#"{{"test":"S9","event":"capture_ended","ok":false,"error":{:?}}}"#, e),
        };
        tracing::info!(target: "spike", "spike: {line}");
    });
    *lb.0.lock().unwrap() = Some(Running { room, stop, thread: Some(thread) });
    Ok(())
}

/// Capture 10 ms stereo i16 chunks until `stop`, the target process exits, or an error.
/// `include=true`: only `pid`'s process tree; `false`: everything EXCEPT `pid`'s tree.
fn capture_loop(
    pid: u32, include: bool, tone: Option<&str>,
    stop: &std::sync::atomic::AtomicBool, mut sink: impl FnMut(Vec<i16>),
) -> Result<(), String> {
    if tone == Some("left") {
        // stereo check: 1 kHz on the left channel only
        let mut n = 0u64;
        while !stop.load(std::sync::atomic::Ordering::Relaxed) {
            let mut c = Vec::with_capacity(960);
            for _ in 0..480 {
                let s = ((n as f32 * 2.0 * std::f32::consts::PI * 1000.0 / 48_000.0).sin() * 8000.0) as i16;
                c.extend([s, 0]);
                n += 1;
            }
            sink(c);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        return Ok(());
    }
    wasapi_capture(pid, include, stop, sink)
}

#[cfg(not(windows))]
fn wasapi_capture(
    _pid: u32, _include: bool, _stop: &std::sync::atomic::AtomicBool, _sink: impl FnMut(Vec<i16>),
) -> Result<(), String> {
    Err("process loopback is Windows-only".into())
}

#[cfg(windows)]
fn wasapi_capture(
    pid: u32, include: bool, stop: &std::sync::atomic::AtomicBool, mut sink: impl FnMut(Vec<i16>),
) -> Result<(), String> {
    wasapi::initialize_mta().ok().map_err(|e| format!("{e:?}"))?;
    let mut client = wasapi::AudioClient::new_application_loopback_client(pid, include)
        .map_err(|e| format!("activate: {e}"))?;
    let fmt = wasapi::WaveFormat::new(16, 16, &wasapi::SampleType::Int, 48_000, 2, None);
    let mode = wasapi::StreamMode::EventsShared { autoconvert: true, buffer_duration_hns: 200_000 };
    client.initialize_client(&fmt, &wasapi::Direction::Capture, &mode).map_err(|e| format!("init: {e}"))?;
    let ev = client.set_get_eventhandle().map_err(|e| format!("event: {e}"))?;
    let cap = client.get_audiocaptureclient().map_err(|e| format!("capture client: {e}"))?;
    client.start_stream().map_err(|e| format!("start: {e}"))?;
    let mut q = std::collections::VecDeque::<u8>::new();
    while !stop.load(std::sync::atomic::Ordering::Relaxed) {
        if include && !process_alive(pid) {
            return Err(format!("target process {pid} exited"));
        }
        if ev.wait_for_event(200).is_err() { continue; }
        cap.read_from_device_to_deque(&mut q).map_err(|e| format!("read: {e}"))?;
        while q.len() >= 960 * 2 {
            let bytes: Vec<u8> = q.drain(..960 * 2).collect();
            sink(bytes.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect());
        }
    }
    client.stop_stream().ok();
    Ok(())
}

#[cfg(windows)]
fn process_alive(pid: u32) -> bool {
    use windows::Win32::System::Threading::{GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
    unsafe {
        let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { return false };
        let mut code = 0u32;
        let ok = GetExitCodeProcess(h, &mut code).is_ok();
        let _ = windows::Win32::Foundation::CloseHandle(h);
        ok && code == 259 // STILL_ACTIVE
    }
}

#[tauri::command]
pub async fn spike_loopback_stop(lb: State<'_, Loopback>) -> Result<(), String> {
    let running = lb.0.lock().unwrap().take();
    if let Some(mut r) = running {
        r.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        if let Some(t) = r.thread.take() { let _ = tokio::task::spawn_blocking(move || t.join()).await; }
        r.room.close().await.map_err(|e| e.to_string())?;
    }
    Ok(())
}
```

`spike_beep_watch(channel_id)` (takes `core: State<Core>`, `lb: State<Loopback>` and
`voice: State<voice::VoiceManager>`):
- connects with a `"a"` ticket. A viewer also gets `#a`; the spike reuses role `a`, because the server-side
  distinction between publish and subscribe isn't needed for the spike. It keeps the `Room` and its tasks in `lb.1`;
  a second call closes the previous one first.
- on `TrackSubscribed` for a `RemoteTrack::Audio` whose `publication.source()` is `TrackSource::ScreenshareAudio`, it
  spawns a task over `NativeAudioStream::new(t.rtc_track(), 48_000, 2)`. This is the same constructor the voice
  receive task uses (`voice/mod.rs` ~line 1368), but stereo. The task:
  - feeds the left channel as f32 into `onset::Detector`;
  - logs each onset as
    `spike: {"test":"S9","event":"beep","t_ms":<SystemTime unix ms of the chunk arrival + offset/48>}`;
  - plays the audio through step zero's mixer, as the real build will (spec table, `#a` row: "plays it through
    Pulse's mixer"). At subscribe it calls `voice.spike_mixer().await`. With `Some(m)` it takes
    `let g = m.generation()` and registers
    `m.add_source(g, &publication.sid().to_string(), owner, SourceKind::Stream)`. Then, per frame, it downmixes to
    mono i16 and calls `m.push_in(g, &sid, &mono)`.
  - on `TrackUnsubscribed`: `m.remove_source(g, &sid)` and abort the task, mirroring the voice path.
  - The `owner` is the identity with the `#a` suffix stripped (`identity.split('#').next()`), so the gain key
    (owner, `Stream`) is the person, as step zero intends. (refreshed 2026-10-22: the owner format is a spike call;
    step zero only ever saw plain user ids.)
  - With `None` (not in voice), it still runs the detector and logs `{"test":"S9","event":"no_mixer"}` once. Nothing
    plays.
  - A voice channel switch bumps the mixer generation (`MixerInput::clear`), and later pushes under the old `g` are
    dropped by design. Re-press **Listen** after switching channels.
- The old "own cpal output" fallback is gone: step zero is merged, so S19 (echo) is unblocked.

Register `Loopback::default()` in `setup` next to `app.manage(voice::mictest::MicTest::default())` (`lib.rs` line 101),
as `#[cfg(feature = "spike")] app.manage(spike::loopback::Loopback::default());`.

**The Windows-only test** (`loopback.rs`, `#[cfg(all(test, windows))]`):

```rust
    #[test]
    fn capture_ends_cleanly_when_the_process_exits() {
        let mut child = std::process::Command::new("cmd").args(["/C", "ping -n 3 127.0.0.1 >NUL"]).spawn().unwrap();
        let stop = std::sync::atomic::AtomicBool::new(false);
        let r = capture_loop(child.id(), true, None, &stop, |_| {});
        let _ = child.wait();
        assert!(r.unwrap_err().contains("exited"));
    }
```

It runs in Windows CI (Task 6). If process-loopback activation itself fails on the CI runner (no audio endpoint),
mark it `#[ignore]` with that reason and have the owner run it once from the spike build's terminal.

- [ ] **Step 4: Build and test**

Run: `cargo test -p pulse-client --lib --features spike spike::onset` → PASS.
Run: `cargo build -p pulse-client --features spike` → builds (Linux: only the WASAPI capture is cfg'd out).
Run: `cargo test -p pulse-client --lib voice::` (no feature) → PASS: step zero's mixer and rx tests are unchanged.
Windows compile and test happen in Task 6's CI run.

- [ ] **Step 5: Commit**

```bash
git add client/src-tauri/src/spike client/src-tauri/src/voice/mod.rs client/src-tauri/src/lib.rs
git commit -m "spike(client): process-loopback stream audio prototype and beep-onset detector

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 5: The debug panel

**Files:**
- Modify: `client/ui/package.json` (add `"livekit-client": "2.22.4"`, exact, to `dependencies`; `pnpm add
  livekit-client@2.22.4 --save-exact` updates `pnpm-lock.yaml` too, which CI installs with `--frozen-lockfile`)
- Modify: `client/ui/src/vite-env.d.ts` (add `readonly VITE_SPIKE?: string` to the existing `ImportMetaEnv`, next to
  `VITE_DEFAULT_SERVER`)
- Create: `client/src-tauri/tauri.spike.conf.json` (spike-only config overlay, Step 0 below)
- Create: `client/ui/src/spike/encodings.ts`, `client/ui/src/spike/encodings.test.ts`
- Create: `client/ui/src/spike/sdp.ts`, `client/ui/src/spike/sdp.test.ts`
- Create: `client/ui/src/spike/sampler.ts`, `client/ui/src/spike/sampler.test.ts`
- Create: `client/ui/src/spike/log.ts`
- Create: `client/ui/src/spike/rooms.ts`
- Create: `client/ui/src/spike/SpikePanel.svelte`
- Create: `client/ui/src/spike/testpattern.html` (the S9 flash-and-beep page, opened in the owner's browser and shared)
- Modify: `client/ui/src/App.svelte` (mount the panel when `import.meta.env.VITE_SPIKE === '1'`; `Ctrl+Shift+F12`
  toggles it)

**Interfaces:**
- Consumes: the Task 3 and Task 4 commands.
- Produces (pure, tested):
  - `encodingFor(res: 720 | 1080 | 1440, fps: 15 | 30 | 60): { maxBitrate: number; maxFramerate: number; contentHint: 'motion' | 'detail'; degradation: 'maintain-framerate' | 'maintain-resolution' }`
    (spec §3.2 table)
  - `addRtxTime(sdp: string, ms: number): string`: adds `;rtx-time=<ms>` to every `a=fmtp:<pt> apt=<n>` line that
    lacks it.
  - `class Ring<T> { constructor(cap: number); push(x: T): void; items(): T[]; }`
  - `strain(prev: Sample, cur: Sample, targetFps: number): { lowFps: boolean; encodeHeavy: boolean; cpuLimited: boolean }`
    (spec §6.2 rules, logged for S21)
  - `logResult(test: string, fn: () => Promise<object>): Promise<void>` always logs a line; on throw it logs
    `{test, ok:false, error}`.

- [ ] **Step 0: The spike config overlay (CSP and the pop-out's permissions)**

(refreshed 2026-10-22: the old plan didn't cover this, and the panel can't work without it. The app's CSP in
`client/src-tauri/tauri.conf.json` is `connect-src ipc: http://ipc.localhost`, so `livekit-client`'s `wss://`
signalling would be blocked. The only capability, `capabilities/default.json`, covers just the `main` window, with no
`set-size` or webview-creation permission, so S8's resize and S18's pop-out would be refused. Rather than loosen the
shipped config, spike builds merge an overlay. `tauri build --config <file>` applies it as a JSON merge patch.)

`client/src-tauri/tauri.spike.conf.json`:

```json
{
  "app": {
    "security": {
      "csp": "default-src 'self' ipc: http://ipc.localhost; img-src 'self' data: blob:; media-src 'self' blob: mediastream:; style-src 'self' 'unsafe-inline'; font-src 'self'; worker-src 'self' blob:; connect-src ipc: http://ipc.localhost wss: https:",
      "capabilities": [
        "default",
        {
          "identifier": "spike",
          "description": "THROWAWAY (screen-share spike): panel resize and the S18 pop-out.",
          "windows": ["main", "popout"],
          "permissions": [
            "core:default",
            "core:window:allow-set-size",
            "core:window:allow-close",
            "core:webview:allow-create-webview-window"
          ]
        }
      ]
    }
  }
}
```

Listing `capabilities` replaces the default behaviour of enabling every file in `capabilities/`, so `"default"` is
named explicitly. Check it with `cd client/src-tauri && pnpm dlx @tauri-apps/cli@^2 build --features spike --config
tauri.spike.conf.json --no-bundle` on Linux (or let Task 6 check it on Windows). A bad permission name fails the build.

- [ ] **Step 1: Write the failing tests**

`encodings.test.ts`:

```ts
import { describe, expect, it } from 'vitest'
import { encodingFor } from './encodings'

describe('encodingFor', () => {
  it('matches the spec table and tunes games for motion', () => {
    expect(encodingFor(720, 60)).toEqual({ maxBitrate: 5_500_000, maxFramerate: 60, contentHint: 'motion', degradation: 'maintain-framerate' })
    expect(encodingFor(1080, 60).maxBitrate).toBe(8_000_000)
    expect(encodingFor(1440, 30).maxBitrate).toBe(7_500_000)
  })
  it('treats 15 fps as text', () => {
    expect(encodingFor(1080, 15)).toEqual({ maxBitrate: 2_500_000, maxFramerate: 15, contentHint: 'detail', degradation: 'maintain-resolution' })
  })
})
```

`sdp.test.ts`:

```ts
import { describe, expect, it } from 'vitest'
import { addRtxTime } from './sdp'

const sdp = ['v=0', 'a=rtpmap:96 H264/90000', 'a=rtpmap:97 rtx/90000', 'a=fmtp:97 apt=96', 'a=fmtp:99 apt=98;rtx-time=200', ''].join('\r\n')

describe('addRtxTime', () => {
  it('adds rtx-time to RTX fmtp lines that lack it and leaves others alone', () => {
    const out = addRtxTime(sdp, 300)
    expect(out).toContain('a=fmtp:97 apt=96;rtx-time=300\r\n')
    expect(out).toContain('a=fmtp:99 apt=98;rtx-time=200\r\n')
    expect(out.split('\r\n').length).toBe(sdp.split('\r\n').length)
  })
})
```

`sampler.test.ts`:

```ts
import { describe, expect, it } from 'vitest'
import { Ring, strain } from './sampler'

describe('Ring', () => {
  it('ring_never_grows_past_its_capacity', () => {
    const r = new Ring<number>(3)
    for (let i = 0; i < 10; i++) r.push(i)
    expect(r.items()).toEqual([7, 8, 9])
  })
})

describe('strain', () => {
  const s = (t: number, frames: number, encSec: number, reason = 'none') =>
    ({ t, framesEncoded: frames, totalEncodeTime: encSec, framesSent: frames, qualityLimitationReason: reason })
  it('flags low fps, heavy encode and cpu limits (spec 6.2)', () => {
    expect(strain(s(0, 0, 0), s(2000, 120, 0.2), 60)).toEqual({ lowFps: false, encodeHeavy: false, cpuLimited: false })
    expect(strain(s(0, 0, 0), s(2000, 80, 0.2), 60).lowFps).toBe(true) // 40 fps < 85% of 60
    expect(strain(s(0, 0, 0), s(2000, 120, 1.5), 60).encodeHeavy).toBe(true) // 12.5 ms/frame > 70% of 16.7
    expect(strain(s(0, 0, 0), s(2000, 120, 0.2, 'cpu'), 60).cpuLimited).toBe(true)
  })
})
```

Run: `cd client/ui && pnpm vitest run src/spike` → FAIL (modules don't exist).

- [ ] **Step 2: Implement the pure helpers**

`encodings.ts`:

```ts
// THROWAWAY (spike) — but this table is the spec's §3.2 and moves to the real build if S2/S4 agree.
const KBPS: Record<number, Record<number, number>> = {
  720: { 15: 1500, 30: 3500, 60: 5500 },
  1080: { 15: 2500, 30: 5000, 60: 8000 },
  1440: { 15: 4000, 30: 7500, 60: 12000 },
}

export function encodingFor(res: 720 | 1080 | 1440, fps: 15 | 30 | 60) {
  const text = fps === 15
  return {
    maxBitrate: KBPS[res][fps] * 1000,
    maxFramerate: fps,
    contentHint: (text ? 'detail' : 'motion') as 'motion' | 'detail',
    degradation: (text ? 'maintain-resolution' : 'maintain-framerate') as 'maintain-framerate' | 'maintain-resolution',
  }
}
```

`sdp.ts`:

```ts
/** S4: without rtx-time, libwebrtc gives up on a lost packet and freezes until a keyframe (~3 s at 1% loss). */
export function addRtxTime(sdp: string, ms: number): string {
  return sdp.replace(/^(a=fmtp:\d+ apt=\d+)(?!.*rtx-time)(.*)$/gm, `$1$2;rtx-time=${ms}`)
}
```

`sampler.ts`:

```ts
export class Ring<T> {
  private buf: T[] = []
  constructor(private cap: number) {}
  push(x: T) {
    this.buf.push(x)
    if (this.buf.length > this.cap) this.buf.shift()
  }
  items(): T[] {
    return [...this.buf]
  }
}

export type Sample = { t: number; framesEncoded: number; totalEncodeTime: number; framesSent: number; qualityLimitationReason: string }

/** Spec §6.2 strain signals between two outbound-rtp samples (the full-layer encoding). */
export function strain(prev: Sample, cur: Sample, targetFps: number) {
  const dt = (cur.t - prev.t) / 1000
  const frames = cur.framesSent - prev.framesSent
  const fps = dt > 0 ? frames / dt : 0
  const encFrames = cur.framesEncoded - prev.framesEncoded
  const encMs = encFrames > 0 ? ((cur.totalEncodeTime - prev.totalEncodeTime) * 1000) / encFrames : 0
  return {
    lowFps: fps < 0.85 * targetFps,
    encodeHeavy: encMs > 0.7 * (1000 / targetFps),
    cpuLimited: cur.qualityLimitationReason === 'cpu',
  }
}
```

`log.ts`:

```ts
import { invoke } from '@tauri-apps/api/core'

export function spikeLog(obj: object) {
  void invoke('spike_log', { line: JSON.stringify({ ...obj, at: Date.now() }) }).catch(() => {})
}

/** Every test logs a line, pass or fail: silence would read as "not run". */
export async function logResult(test: string, fn: () => Promise<object>) {
  try {
    spikeLog({ test, ok: true, ...(await fn()) })
  } catch (e) {
    spikeLog({ test, ok: false, error: String(e) })
  }
}
```

Run: `pnpm vitest run src/spike` → PASS.

- [ ] **Step 3: Rooms and publishing (`rooms.ts`)**

```ts
import { Room, Track, VideoPreset, type RemoteTrack, type ScreenShareCaptureOptions, type TrackPublishOptions } from 'livekit-client'
import { invoke } from '@tauri-apps/api/core'
import { encodingFor } from './encodings'
import { addRtxTime } from './sdp'
import { spikeLog } from './log'

type Ticket = { url: string; token: string }
const ticket = (channelId: string, role: 'v' | 'v2') => invoke<Ticket>('spike_ticket', { channelId, role })

export type PublishOpts = { res: 720 | 1080 | 1440; fps: 15 | 30 | 60; lowLayer: boolean; codec: 'h264' | 'av1' }

export async function connect(channelId: string, role: 'v' | 'v2', rtxTime: boolean): Promise<Room> {
  const t0 = performance.now()
  const room = new Room({ adaptiveStream: false, dynacast: true })
  if (rtxTime) patchAnswers(room)
  const tk = await ticket(channelId, role)
  await room.connect(tk.url, tk.token, { autoSubscribe: false })
  spikeLog({ test: 'S12', event: 'connected', role, ms: Math.round(performance.now() - t0) })
  return room
}

/** S4(b): munge every answer we create so the viewer asks for rtx-time=300. */
function patchAnswers(room: Room) {
  const orig = RTCPeerConnection.prototype.setLocalDescription
  RTCPeerConnection.prototype.setLocalDescription = function (desc?: RTCLocalSessionDescriptionInit) {
    if (desc?.type === 'answer' && desc.sdp) desc = { ...desc, sdp: addRtxTime(desc.sdp, 300) }
    return orig.call(this, desc as RTCLocalSessionDescriptionInit)
  }
  void room
}

export async function publish(room: Room, o: PublishOpts) {
  const e = encodingFor(o.res, o.fps)
  const capture: ScreenShareCaptureOptions = {
    audio: false,
    resolution: { width: (o.res * 16) / 9, height: o.res, frameRate: o.fps },
    contentHint: e.contentHint,
    selfBrowserSurface: 'exclude',
  }
  const pubOpts: TrackPublishOptions = {
    videoCodec: o.codec,
    screenShareEncoding: { maxBitrate: e.maxBitrate, maxFramerate: e.maxFramerate },
    screenShareSimulcastLayers: o.lowLayer ? [new VideoPreset(640, 360, 500_000, 30)] : [],
    degradationPreference: e.degradation,
    simulcast: o.lowLayer,
  }
  const t0 = performance.now()
  const pubs = await room.localParticipant.createScreenTracks(capture)
  const video = pubs.find((t) => t.kind === Track.Kind.Video)!
  const label = video.mediaStreamTrack.label
  const settings = video.mediaStreamTrack.getSettings()
  const pub = await room.localParticipant.publishTrack(video, pubOpts)
  spikeLog({ test: 'S8', event: 'picked', label, displaySurface: settings.displaySurface, w: settings.width, h: settings.height, pickMs: Math.round(performance.now() - t0) })
  return pub
}

export async function watch(room: Room, streamer: string, el: HTMLVideoElement) {
  const t0 = performance.now()
  return new Promise<RemoteTrack>((resolve, reject) => {
    const p = room.remoteParticipants.get(`${streamer}#v`)
    const pub = p?.getTrackPublication(Track.Source.ScreenShare)
    if (!pub) return reject(new Error(`${streamer} is not publishing`))
    pub.setSubscribed(true)
    const check = () => {
      if (!pub.track) return requestAnimationFrame(check)
      pub.track.attach(el)
      el.requestVideoFrameCallback(() => {
        spikeLog({ test: 'S12', event: 'first_frame', ms: Math.round(performance.now() - t0) })
        resolve(pub.track!)
      })
    }
    check()
  })
}
```

Check these `livekit-client` 2.22.4 names against its `index.d.ts`: `createScreenTracks`, `screenShareEncoding`,
`screenShareSimulcastLayers`, `degradationPreference`, `ScreenShareCaptureOptions.contentHint`. If one differs, use
the real one. The research (`screenshare-deep-codecs.md`) cites the exact source files.

- [ ] **Step 4: The panel (`SpikePanel.svelte`)**

A fixed overlay with collapsible sections. Each button calls a runner wrapped in `logResult`. Sections, with what
each button does:

- **Setup:** shows `navigator.userAgent` and logs it with the GPU line the owner types in (`S0`). Shows the current
  voice channel id, `voice.channelId` from `lib/voice.svelte.ts` (set by the Rust `voice://event` state events; `null`
  when not in voice); every test uses it. (refreshed 2026-10-22: `app.state` is the gateway store and has no voice
  channel.)
- **S1 / S5 Encode:**
  - resolution, fps, low-layer and codec selectors, plus **Go live** (`connect('v')` + `publish`) and **Stop**.
  - Every 2 s it logs outbound-rtp per encoding:
    - `encoderImplementation`, `powerEfficientEncoder`, `framesPerSecond`;
    - `totalEncodeTime` delta per frame, `qualityLimitationReason`;
    - the codec's `sdpFmtpLine`, `targetBitrate` and `bytesSent` delta.
  - It also logs `strain()` (S21).
  - A **Mark** text box logs `{test:'S2',mark:'<text>'}`, so PresentMon runs can be lined up with the log.
- **S4 Viewer:** **Watch** (`connect('v', rtxTime)` + `watch`).
  - toggles: rtx-time on/off; Data saver (`setVideoQuality(VideoQuality.LOW)`).
  - Every 2 s it logs inbound-rtp: `framesPerSecond`, `freezeCount`, `totalFreezesDuration`, `frameWidth`,
    `frameHeight`, `decoderImplementation`, `powerEfficientDecoder`, `jitterBufferDelay` delta, `packetsLost` delta,
    `nackCount`, `pliCount`.
- **S6 Background:** logs `document.visibilityState`, `document.hasFocus()` and the measured rVFC fps every 2 s,
  continuously while the section is enabled.
  - (refreshed 2026-10-22) Since the optimisation run, main sets WebView2's Low memory target when minimised, or
    after 120 s unfocused, and Normal again on focus (`webview_memory.rs`, `BACKGROUND_AFTER`). Each change logs a
    `webview memory target` line. S6 runs must last past 120 s in the background, so the result covers the state
    the real app sits in. The results write-up reads those lines next to the fps samples. If Low hurts decode or
    encode, that's a spec finding (the real build would hold Normal while streaming or watching), not a spike change.
- **S7 Decode:** a `getUserMedia({audio:true})` unlock button (WebView2 hides decoder stats without a media
  permission), then the S4 stats.
- **S3 Soak:** **Start soak** logs, every 30 s, `spike_os_stats()` plus `performance.memory` (if present) plus the S1
  and S4 summary into one line `{test:'S3',...}`. It keeps only the latest sample and a `Ring(120)`.
  - **Cycle ×20:** stop and start the share 20 times with `replaceTrack` (`localTrack.replaceTrack(newTrack)`) and
    then 20 times with unpublish and republish, logging `spike_os_stats()` after each.
- **S8 Picker:**
  - **Set window height 560 / 640** (Tauri `getCurrentWindow().setSize(new LogicalSize(w, h))`; needs the overlay's
    `core:window:allow-set-size`; 560 is the main window's `min_inner_size` height, `lib.rs`), then **Pick**;
  - logs the label and settings;
  - **Find PID by title** calls `spike_window_pid(title)` with the title parsed from the label, logs the result
    (gap 4), and fills the S9 PID box.
- **S9 / S19 Audio:**
  - mode include/exclude, the PID box, and tone left/none;
  - **Start stream audio** (`spike_loopback_start`) and **Stop**;
  - on the viewer, **Listen** (`spike_beep_watch`);
  - on the viewer, **Measure A/V**: watches the stream `<video>` with `requestVideoFrameCallback`, samples the
    centre 16×16 pixels into a canvas each frame, and logs a `flash` event (`t_ms` = `Date.now()` at
    `expectedDisplayTime`) whenever brightness jumps from <30 to >200. Skew = beep `t_ms` − flash `t_ms`, paired
    nearest within 500 ms. The results write-up computes it from the log lines.
- **S12 Lifecycle:** **Kill renderer**, which navigates to `edge://kill` (`location.href = 'edge://kill'`) and fires
  ProcessFailed. **Reconnect test** calls `room.disconnect()` then `connect` again 5 times, logging timings.
- **S13 Preview:** every 15 s, `ImageCapture(track).grabFrame()` → `OffscreenCanvas(480,270)` → `convertToBlob({type:'image/webp',quality:0.7})`.
  It logs the size and the time taken. Runs while live.
- **S18 Pop-out:** **Pop out** opens a `WebviewWindow` labelled `popout` loading `index.html?spike-popout=<streamer>`
  (allowed by the overlay's `spike` capability). It gets wry's default browser args and Tauri's default data
  directory, not the main window's per-profile one. Tauri requires different data directories for different args, so
  that's what keeps it legal. But it means a second WebView2 browser process, whose start-up is part of the measured
  blip. Note that in the results.
  That page connects as `v2`, watches, and logs `first_frame` with a shared epoch (`Date.now()`). The main window
  unsubscribes on the popout's `first_frame` event (Tauri `emit`) and logs `handover` with the last-frame and
  first-frame times. The blip is the gap.
- **S20 Capture check:** **Black-frame watch** samples the local preview track every 500 ms (64×36 canvas): average
  luma <8 or an unchanged hash for 3 s logs `{test:'S20',event:'black_or_frozen'}`.

`App.svelte`: when `import.meta.env.VITE_SPIKE === '1'`, import the panel lazily
(`const SpikePanel = (await import('./spike/SpikePanel.svelte')).default`, in an `onMount` like the existing ones).
Render it after the `{#if screen === ...}` block. Toggle it from a `<svelte:window onkeydown>` handler on
`Ctrl+Shift+F12` (`e.ctrlKey && e.shiftKey && e.code === 'F12'`). Handle `?spike-popout=` by rendering only the
pop-out viewer.
- Keybinds v2 doesn't get in the way: the Windows low-level hooks (`hotkeys.rs`) always pass every key on via
  `CallNextHookEx`, so the webview still sees `Ctrl+Shift+F12` even if the owner has bound it.
- While a bind is being recorded, `KeybindSettings.svelte` listens with `onkeydowncapture` on the window and calls
  `preventDefault()` + `stopPropagation()`. So a plain (bubble-phase) `onkeydown` toggle never fires mid-record, which
  is what we want. Don't register the toggle in the capture phase.

`testpattern.html`: a standalone page. It shows a black screen that flashes white for 100 ms every 2 s, and plays a
100 ms 1 kHz beep through Web Audio at the same `performance.now()` tick. The owner opens it in Edge or Chrome and
shares that window for S9.

- [ ] **Step 5: Check and build**

Run: `cd client/ui && pnpm test && pnpm check && VITE_SPIKE=1 pnpm build`
Expected: PASS. The normal `pnpm build` (no `VITE_SPIKE`) must not include `livekit-client`. Check with
`grep -li livekit dist/assets/*.js` after a plain `pnpm build` (no output: no shipped UI source mentions "livekit"
today) to confirm the panel is tree-shaken.

- [ ] **Step 6: Commit**

```bash
git add client/ui client/src-tauri/tauri.spike.conf.json
git commit -m "spike(ui): hidden screen-share debug panel (S1-S21 runners, all results logged)

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 6: Windows spike build in CI

**Files:**
- Modify: `.github/workflows/release-windows.yml`

**Interfaces:**
- Produces: `gh workflow run release-windows.yml --ref screenshare-spike -f spike=true` builds a kit with the spike
  installer.

- [ ] **Step 1: Add the input and wire it**

```yaml
on:
  push:
    tags: ['v*']
  workflow_dispatch:
    inputs:
      spike:
        description: Build the screen-share spike (debug panel + loopback prototype)
        type: boolean
        default: false
```

This replaces the existing `on:` block. It has the same `push: tags: ['v*']` trigger, plus the input on the existing
bare `workflow_dispatch:`. Then:
- In `env:`, add `VITE_SPIKE: ${{ inputs.spike && '1' || '' }}`. `LIVEKIT_VERSION` is already `1.13.10` on main, so
  leave it. (refreshed 2026-10-22: the bump the old plan asked for has already happened.)
  - `VITE_DEFAULT_SERVER` and `PULSE_COMMIT` stay as they are. The spike kit's login screen starts on the real server,
    which is what the owner's runs need.
- Change the build step (`working-directory: client/src-tauri`) from `pnpm dlx @tauri-apps/cli@^2 build` to
  `pnpm dlx @tauri-apps/cli@^2 build ${{ inputs.spike && '--features spike --config tauri.spike.conf.json' || '' }}`.
  Its `beforeBuildCommand` (`pnpm --dir ui build`) inherits `VITE_SPIKE` from the job env.
- Add a test step after the Tauri build (so `client/ui/dist` exists for `generate_context!`) and before
  `Assemble test kit`:
  `- if: inputs.spike` / `run: cargo test -p pulse-client --lib --features spike spike:: -- --include-ignored`. If
  the loopback test can't activate audio on the runner, drop `--include-ignored` (the owner runs it locally).
- `Assemble test kit` is unchanged. It still copies `target/release/pulse-app.exe`, `pulse-app-server.exe`, the NSIS
  installer and LiveKit, and the artifact is still `pulse-windows-kit`. Its `pulse-app-server.exe` is built by the
  separate `cargo build --release -p pulse-server` step, so the kit's local server has the spike route too (off
  unless `PULSE_SPIKE=1`).
- `ci.yml` only runs on pull requests and pushes to `main`, so pushing `screenshare-spike` runs no CI. Lint locally
  (Task 3 Step 4).

- [ ] **Step 2: Run it**

```bash
git add .github/workflows/release-windows.yml
git commit -m "spike(ci): build the screen-share spike kit on demand

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
git push -u origin screenshare-spike
gh workflow run release-windows.yml --ref screenshare-spike -f spike=true
gh run watch "$(gh run list --workflow release-windows.yml --branch screenshare-spike --limit 1 --json databaseId -q '.[0].databaseId')"
```

Expected: the run succeeds and the artifact `pulse-windows-kit` exists. Fix Windows-only compile errors (the
`windows`/`wasapi`/`webview2-com` paths from Tasks 3 and 4) here, one commit per fix.

The spike server image is a separate step, and it's a prod change that needs the owner's OK (Global Constraints):
`gh workflow run release-server.yml --ref screenshare-spike`, then `just deploy <spike-sha7>` from the spike worktree.
Remember that this run also moves `:latest`.

---

### Task 7: The owner's protocol

**Files:**
- Create: `docs/superpowers/research/screenshare-spike-protocol.md`

- [ ] **Step 1: Write the protocol**

A numbered, copy-paste script for the owner. No IP addresses or names. It covers:

1. **Install:**
   - the spike installer from the kit on both PCs;
   - PresentMon 2.x (or CapFrameX) and clumsy 0.3 on the streamer PC.
2. **Before each session:**
   - once, before the first session: ask Claude to deploy the spike server image (`just deploy <spike-sha7>`, Global
     Constraints). It has to happen before any codec switch, because `just deploy` re-copies `livekit.yaml`;
   - ask Claude to run `just spike-server on` (and `just spike-codecs on|off` per S1);
   - join voice on both PCs. The viewer must be in voice for S9/S19: **Listen** plays the stream audio through the
     voice mixer, and with no call it only detects beeps;
   - press `Ctrl+Shift+F12`;
   - in Setup, type the GPU and driver version.
3. **One block per test, in this order:** S1 → S2 → S5 → S4 → S7 → S8 → S9 → S19 → S20 → S12 → S13 → S18 → S6 → S3
   (S3 last: 2 hours). Each block has:
   - the exact buttons;
   - the external tool steps:
     - **S2:** PresentMon 3 × 60 s per configuration, with a Mark in the panel before each;
     - **S4:** clumsy profiles: 1% drop, 5% drop, 3 Mbps throttle, 100 ms lag with ±30 ms jitter;
     - **S1:** Task Manager "Video Encode";
     - **S3:** Task Manager "Dedicated GPU memory" read at 0, 30, 60, 90 and 120 min, typed into a Mark;
   - **"File a bug report now, titled `spike S<n>`"** at the end. Reports carry the log tail (`TAIL_BYTES`, 256 KB,
     `client/src-tauri/src/reports/mod.rs`), and one per test keeps each under that.
   - **S6:** stay in the background for more than 2 minutes. That's when the app drops WebView2 to its Low memory
     target (Task 5, S6).
4. **What to say in the report text:** the game, display mode (borderless/exclusive), anything seen by eye (stutter,
   black, echo).
5. **When done:** ask Claude to run `just spike-server off` and `just spike-codecs off`, then put main's server back
   (`just deploy <main-sha7>` from `main`, and re-run `release-server.yml` on `main` so `:latest` is main again).

- [ ] **Step 2: Commit**

```bash
git add docs/superpowers/research/screenshare-spike-protocol.md
git commit -m "docs(screenshare): the owner's spike protocol

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 8: Results and spec update (after the owner's runs)

**Files:**
- Create: `docs/superpowers/research/screenshare-spike-results.md`
- Modify: `docs/superpowers/specs/2026-10-15-screenshare-design.md` (only where results disagree)

- [ ] **Step 1: Pull the reports**

`just reports`, then `just report <id> --log` for each `spike S<n>` report, saved to the scratchpad. Extract with
`grep 'spike: '` and parse the JSON lines.

- [ ] **Step 2: Write the results**

`screenshare-spike-results.md` has one table row per test:
- test;
- PC and GPU;
- WebView2 version;
- measured value(s);
- the pass bar (crosscheck §4 or spec §6.2/§4.3);
- PASS / FAIL;
- what it changes.

S9's skew is computed from beep and flash pairs as the median and p90. S2 is computed from PresentMon CSVs as the
average and 1% low change per configuration.

- [ ] **Step 3: Update the spec where reality differs**

Examples:
- bitrates in §3.2;
- step-down thresholds in §6.2;
- the S8 fallback to EXCLUDE;
- the S1 outcome (if hardware H.264 fails, the native path moves into v1: stop and bring that to the owner before any
  planning).

- [ ] **Step 4: Commit, and hand the decisions to the owner**

```bash
git add docs/superpowers/research/screenshare-spike-results.md docs/superpowers/specs/2026-10-15-screenshare-design.md
git commit -m "docs(screenshare): spike results and spec updates

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

Then:
- revert prod: `just spike-server off`, `just spike-codecs off`, then `just deploy <main-sha7>` from `main` (this also
  restores main's `deploy/compose.yml` without `PULSE_SPIKE`) and re-run `release-server.yml` on `main` for
  `:latest`;
- ask the owner to approve the updated spec;
- write the build-phase plans (server, streamer, viewer, adaptation, failure handling) against the measured numbers.
