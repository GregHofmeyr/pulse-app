# Screen share Windows spike — Implementation Plan

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
  - a prototype stream-audio path (WASAPI process loopback published into the stream room);
  - beep-onset detection for the A/V sync test.
- **Client, UI:** a hidden debug panel (`Ctrl+Shift+F12`, spike builds only) built on `livekit-client`, with one
  section per test. It logs `spike:` JSON lines.

None of this merges to `main`. The **results document** and the **spec updates** are what we keep.

**Tech Stack:**
- Rust: axum (server); Tauri 2, `livekit` 0.9.3, the `wasapi` crate and `windows` crate (client).
- UI: Svelte 5, `livekit-client` 2.22.4.
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
| `server/src/config.rs`, `server/src/lib.rs` | The `spike` flag; mount the router |
| `deploy/livekit.codecs.yaml` | The `room.enabled_codecs` block for S1(b) |
| `justfile` | `spike-codecs on|off`, `spike-server on|off` |
| `client/src-tauri/Cargo.toml` | The `spike` feature; `wasapi`, `windows` deps (Windows-only, optional) |
| `client/src-tauri/src/spike/mod.rs` | Commands: `spike_log`, `spike_ticket`, `spike_os_stats`, `spike_window_pid` |
| `client/src-tauri/src/spike/loopback.rs` | Process loopback capture to `NativeAudioSource`; the `#a` room connection |
| `client/src-tauri/src/spike/onset.rs` | Beep-onset detector (pure, tested) |
| `client/src-tauri/src/lib.rs` | Extra WebView2 args; renderer-crash hook; register spike commands under `cfg(feature = "spike")` |
| `client/ui/src/spike/*.ts`, `client/ui/src/spike/SpikePanel.svelte` | The debug panel and its pure helpers (tested with vitest) |
| `.github/workflows/release-windows.yml` | A `spike` input that builds with the feature and `VITE_SPIKE=1` |
| `docs/superpowers/research/screenshare-spike-protocol.md` | The owner's step-by-step script |
| `docs/superpowers/research/screenshare-spike-results.md` | Results, written after the runs |

---

### Task 1: Server spike tickets (env-gated)

**Files:**
- Create: `server/src/spike.rs`
- Modify: `server/src/config.rs` (add `pub spike: bool`, read from `PULSE_SPIKE`)
- Modify: `server/src/lib.rs:25-35` (merge `spike::router()`)
- Modify: `server/src/testing.rs` (`test_config()` sets `spike: false`)
- Test: `server/tests/spike.rs`

**Interfaces:**
- Produces: `POST /spike/stream/{channel}/ticket` with body `{"role":"v"|"v2"|"a"}`, returning
  `VoiceTokenResponse { url, token }`.
  - Room: `<channel>~s`.
  - Identity: `<uid>#<role>`.
  - Grants: `room_join`; `can_subscribe: true`; `can_publish: true` with `can_publish_sources: ["screen_share"]` for
    `v`, `[]` (subscribe-only) for `v2`, and `["screen_share_audio"]` for `a`.
  - Requires: server membership, and the caller in that channel's voice (`s.voice` state).
  - Returns 404 when `spike` is false.

- [ ] **Step 1: Write the failing tests**

`server/tests/spike.rs`:

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
    let res = testing::post_json(&app, &tok, &format!("/spike/stream/{ch}/ticket"), &json!({"role":"v"})).await;
    assert_eq!(res.status(), 404);
}

#[tokio::test]
async fn spike_tickets_only_name_the_stream_room() {
    let mut cfg = testing::test_config();
    cfg.spike = true;
    let app = testing::spawn_with(cfg.clone()).await;
    let (uid, tok) = testing::register(&app, "alex").await;
    let ch = lounge(&app, &tok).await;
    testing::mark_in_voice(&app, ch.parse().unwrap(), uid); // test helper, added in step 3
    for (role, sources) in [("v", vec!["screen_share"]), ("a", vec!["screen_share_audio"]), ("v2", vec![])] {
        let res = testing::post_json(&app, &tok, &format!("/spike/stream/{ch}/ticket"), &json!({"role":role})).await;
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
    let res = testing::post_json(&app, &tok, &format!("/spike/stream/{ch}/ticket"), &json!({"role":"v"})).await;
    assert_eq!(res.status(), 403);
}
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p pulse-server --test spike`
Expected: compile errors (`spike` field, `mark_in_voice` and the route don't exist).

- [ ] **Step 3: Implement**

`server/src/config.rs`: add `pub spike: bool` to `Config`, read in `from_env` as
`spike: std::env::var("PULSE_SPIKE").is_ok_and(|v| v == "1")`.

`server/src/testing.rs`:
- in `test_config()` set `spike: false`;
- add the helper:

```rust
/// Pretend the user joined voice (no LiveKit in unit tests).
pub fn mark_in_voice(app: &TestApp, channel: pulse_protocol::ids::ChannelId, user: pulse_protocol::ids::UserId) {
    app.state.voice.join(channel, user, "PA_test");
}
```

`TestApp` must expose `state`. If it doesn't, add `pub state: AppState` where `spawn_with` builds it. Before relying on
the name, check that `voice::state` has an "is this user in this channel" query; the webhook's `s.voice.join` and
`s.voice.declared` show the shape. If it isn't there, add `pub fn is_in(&self, channel, user) -> bool` with its own
unit test in `voice/state.rs`.

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
    let _ch = channel_for(&s.db, me, id).await?;
    if !s.voice.is_in(id, me) {
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

If `AppError::NotFound` doesn't exist, add it, mapping to 404, like the existing variants in `error.rs`. In `lib.rs`
add `pub mod spike;` and `.merge(spike::router())`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p pulse-server --test spike` and then `cargo test -p pulse-server`
Expected: PASS. The webhook already ignores `~s` rooms because `"<id>~s"` doesn't parse as a `ChannelId`. Add this
assertion to `spike_tickets_only_name_the_stream_room` to pin it:
`assert!(format!("{ch}~s").parse::<pulse_protocol::ids::ChannelId>().is_err());`

- [ ] **Step 5: Commit**

```bash
git add server/src/spike.rs server/src/config.rs server/src/lib.rs server/src/testing.rs server/src/error.rs server/tests/spike.rs server/src/voice/state.rs
git commit -m "spike(server): env-gated stream-room tickets (never the voice room)

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 2: LiveKit codec switch and deploy recipes

**Files:**
- Create: `deploy/livekit.codecs.yaml`
- Modify: `justfile` (append the recipes)
- Modify: `deploy/compose.yml`: pass `PULSE_SPIKE` through to `pulse-server` (`PULSE_SPIKE: ${PULSE_SPIKE:-0}`)

**Interfaces:**
- Produces:
  - `just spike-codecs on|off`: appends the block to prod's `livekit.yaml` and restarts LiveKit, or restores the repo
    copy;
  - `just spike-server on|off`: sets `PULSE_SPIKE` in prod `.env` and recreates `pulse-server`.

- [ ] **Step 1: Write the codec block**

`deploy/livekit.codecs.yaml`. This is LiveKit v1.13 `config.go`'s full default list, plus strict Main and Baseline
H.264. Never trim it: dropping `audio/red` or `video/rtx` silently breaks voice loss recovery and video
retransmission.

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

Validate it with LiveKit itself, so the YAML keys are checked rather than guessed:
`docker run --rm -v "$PWD/deploy:/c" livekit/livekit-server:v1.13.10 --config /c/livekit.yaml --dev --bind 127.0.0.1 &`
won't validate the appended block on its own. Instead concatenate:
`cat deploy/livekit.yaml deploy/livekit.codecs.yaml > /tmp/lk-spike.yaml`. Then run
`docker run --rm --network host -e LIVEKIT_KEYS="k: secretsecretsecretsecretsecretsecret" -v /tmp/lk-spike.yaml:/c.yaml livekit/livekit-server:v1.13.10 --config /c.yaml`
and check that the log shows no config error before it starts listening. Stop it with Ctrl+C. If LiveKit rejects
`fmtp_line`, check its `config.go` `CodecSpec` field names for v1.13.10 and fix the key.

- [ ] **Step 2: Add the recipes**

Append to `justfile`:

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

Run: `just deploy-check` (validates compose and shellcheck).
Expected: PASS.

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
- Modify: `client/src-tauri/src/lib.rs` (`create_main_window`, the `invoke_handler` list)
- Modify: `client/src-tauri/src/api.rs` (add `spike_ticket`)
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

In `client/src-tauri/Cargo.toml`:

```toml
[features]
dfn-ll = ["deep_filter/default-model-ll"]
# THROWAWAY: the screen-share spike build (debug panel commands, loopback prototype).
spike = ["dep:wasapi", "dep:windows"]

[target.'cfg(windows)'.dependencies]
wasapi = { version = "0.26", optional = true }
windows = { version = "0.61", optional = true, features = [
  "Win32_Foundation", "Win32_System_ProcessStatus", "Win32_System_Threading",
  "Win32_UI_WindowsAndMessaging", "Win32_System_Diagnostics_ToolHelp",
] }
```

Keep an existing `[target.'cfg(windows)'.dependencies]` table if there is one (merge, don't duplicate). Match the
`windows` version to the one already in `Cargo.lock` (`grep -A1 'name = "windows"' Cargo.lock`) to avoid a second
copy.

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

Run: `cargo test -p pulse-client --features spike spike::`
Expected: FAIL (`webview_args` not found).

- [ ] **Step 3: Implement**

`client/src-tauri/src/spike/mod.rs`:

```rust
//! THROWAWAY (screen-share spike): commands for the hidden debug panel. Results are `spike:` log lines,
//! so every bug report carries them.

#[cfg(windows)]
pub mod loopback;
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
    let (api, token) = core.api_and_token().await?;
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

The `windows` crate path names shift between versions (for example `BOOL` lives in `windows::core` from 0.61, and in
`Win32::Foundation` before). Compile on Windows CI (Task 6) and fix paths to the pinned version. Don't change the
logic.

In `api.rs`, add next to `voice_token`:

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

`core.api_and_token()`: use whatever `join_voice` in `commands.rs` uses to get the `Api` and session token, and copy
those two lines rather than inventing a helper. If there's no shared helper, inline them in `spike_ticket`.

In `lib.rs`:
- add `#[cfg(feature = "spike")] pub mod spike;`.
- **WebView2 args:** in `create_main_window`, before `.build()`:

```rust
    #[cfg(all(windows, feature = "spike"))]
    let win = win.additional_browser_args(&spike::webview_args(
        // wry's default: keep it, or these features come back on
        "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection",
        std::env::var("PULSE_WEBVIEW2_ARGS").ok().as_deref(),
    ));
```

  `win` is the builder at that point; restructure into `let mut b = WebviewWindowBuilder::new(...)...;` then
  `#[cfg] { b = b.additional_browser_args(...) }` then `let win = b.build()?;`.
- **Renderer crash (S12), Windows:** after the window is built, mirroring the Linux block:

```rust
    #[cfg(all(windows, feature = "spike"))]
    win.with_webview(|wv| {
        use webview2_com::ProcessFailedEventHandler;
        let core = unsafe { wv.controller().CoreWebView2() }.expect("core webview");
        let mut token = Default::default();
        let _ = unsafe {
            core.add_ProcessFailed(
                &ProcessFailedEventHandler::create(Box::new(|_, args| {
                    let kind = args.map(|a| unsafe { a.ProcessFailedKind() }.map(|k| k.0).unwrap_or(-1));
                    tracing::error!(target: "spike", "spike: {{\"test\":\"S12\",\"event\":\"renderer_failed\",\"kind\":{}}}", kind.unwrap_or(-1));
                    Ok(())
                })),
                &mut token,
            )
        };
    })?;
```

  `webview2-com` 0.39.1 is already in `Cargo.lock` through wry. Add it as a direct optional Windows dependency in the
  `spike` feature at the same version.
- **Register the commands:** `tauri::generate_handler!` can't take `cfg` attributes on individual entries. Build two
  handler lists:
  - `#[cfg(feature = "spike")] let builder = builder.invoke_handler(generate_handler![...all existing..., spike::spike_log, spike::spike_ticket, spike::spike_os_stats, spike::spike_window_pid, spike::loopback::spike_loopback_start, spike::loopback::spike_loopback_stop, spike::loopback::spike_beep_watch]);`
  - `#[cfg(not(feature = "spike"))]` keeps the original list.

  (The loopback commands come in Task 4; add them to the list then.)

- [ ] **Step 4: Run tests and both builds**

Run: `cargo test -p pulse-client --features spike spike::` → PASS.
Run: `cargo build -p pulse-client` and `cargo build -p pulse-client --features spike` (on Linux, the Windows-only code
is cfg'd out) → both build.

- [ ] **Step 5: Commit**

```bash
git add client/src-tauri
git commit -m "spike(client): log/ticket/OS-stats commands, WebView2 args, renderer-crash hook

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 4: Stream-audio prototype: process loopback, the `#a` connection, beep onset

**Files:**
- Create: `client/src-tauri/src/spike/onset.rs` (pure, tested on any OS)
- Create: `client/src-tauri/src/spike/loopback.rs` (Windows)
- Test: `client/src-tauri/src/spike/onset.rs`; `client/src-tauri/src/spike/loopback.rs` (Windows-only test)

**Interfaces:**
- Consumes:
  - Task 3's `spike_ticket`;
  - step zero's `Mixer::add_source(source, owner, SourceKind::Stream)`, if step zero is merged. If it isn't, the
    prototype plays through its own cpal output instead (see Step 3).
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

Run: `cargo test -p pulse-client --features spike spike::onset` → FAIL (no `Detector`).

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

Run: `cargo test -p pulse-client --features spike spike::onset` → PASS.

- [ ] **Step 3: Implement loopback publish and listen (Windows)**

`client/src-tauri/src/spike/loopback.rs` outline (the code goes in as written below; where a `wasapi` 0.26 signature
differs, follow that crate's `examples/loopback_application.rs` or equivalent, which is the reference):

```rust
//! THROWAWAY (S9, S19): stream audio via WASAPI process loopback, published as `<uid>#a` into `<ch>~s`.

use std::sync::{Arc, Mutex};
use livekit::options::TrackPublishOptions;
use livekit::prelude::*;
use livekit::webrtc::audio_source::native::NativeAudioSource;
use livekit::webrtc::audio_source::{AudioSourceOptions, RtcAudioSource};
use livekit::webrtc::prelude::AudioFrame;
use pulse_protocol::ids::ChannelId;
use tauri::State;

use crate::commands::Core;

#[derive(Default)]
pub struct Loopback(Mutex<Option<Running>>);
struct Running { room: Room, stop: Arc<std::sync::atomic::AtomicBool>, thread: Option<std::thread::JoinHandle<()>> }

#[tauri::command]
pub async fn spike_loopback_start(
    core: State<'_, Core>, lb: State<'_, Loopback>,
    channel_id: ChannelId, mode: String, pid: Option<u32>, tone: Option<String>,
) -> Result<(), String> {
    let t = super::spike_ticket(core, channel_id, "a".into()).await.map_err(|e| e.to_string())?;
    let (room, _ev) = Room::connect(&t.url, &t.token, RoomOptions::default()).await.map_err(|e| e.to_string())?;
    // stereo 48 kHz (needs the rust-sdks #1018 patch to really go out as stereo; S9 checks L≠R on the far end)
    let src = NativeAudioSource::new(AudioSourceOptions::default(), 48_000, 2, 100);
    let track = LocalAudioTrack::create_audio_track("stream-audio", RtcAudioSource::Native(src.clone()));
    room.local_participant()
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

`spike_beep_watch(channel_id)`:
- connects with a `"a"` ticket. A viewer also gets `#a`; the spike reuses role `a`, because the server-side
  distinction between publish and subscribe isn't needed for the spike.
- on `TrackSubscribed` for an audio track with source `ScreenshareAudio`, it spawns a `NativeAudioStream` (48 kHz,
  2 channels), feeds samples as f32 (left channel) into `onset::Detector`, and logs each onset as
  `spike: {"test":"S9","event":"beep","t_ms":<SystemTime unix ms of the chunk arrival + offset/48>}`.
- It plays the audio through the voice mixer: if step zero is merged, `add_source(sid, owner, SourceKind::Stream)`
  and push mono-downmixed chunks. Otherwise it opens a separate cpal default output. S19 (echo) needs the mixer path,
  so S19 waits for step zero.

Register `Loopback::default()` with `.manage(...)` under `cfg(feature = "spike")`.

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

Run: `cargo test -p pulse-client --features spike spike::onset` → PASS.
Run: `cargo build -p pulse-client --features spike` → builds (Linux: the loopback module is cfg'd out).
Windows compile and test happen in Task 6's CI run.

- [ ] **Step 5: Commit**

```bash
git add client/src-tauri/src/spike
git commit -m "spike(client): process-loopback stream audio prototype and beep-onset detector

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 5: The debug panel

**Files:**
- Modify: `client/ui/package.json` (add `"livekit-client": "2.22.4"`, exact)
- Modify: `client/ui/src/vite-env.d.ts` (`readonly VITE_SPIKE?: string`)
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

Run: `cd client/ui && pnpm test -- spike` → FAIL (modules don't exist).

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

Run: `pnpm test -- spike` → PASS.

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
  voice channel id from the store (`app.state`); every test uses it.
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
- **S7 Decode:** a `getUserMedia({audio:true})` unlock button (WebView2 hides decoder stats without a media
  permission), then the S4 stats.
- **S3 Soak:** **Start soak** logs, every 30 s, `spike_os_stats()` plus `performance.memory` (if present) plus the S1
  and S4 summary into one line `{test:'S3',...}`. It keeps only the latest sample and a `Ring(120)`.
  - **Cycle ×20:** stop and start the share 20 times with `replaceTrack` (`localTrack.replaceTrack(newTrack)`) and
    then 20 times with unpublish and republish, logging `spike_os_stats()` after each.
- **S8 Picker:**
  - **Set window height 560 / 640** (Tauri `getCurrentWindow().setSize`), then **Pick**;
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
- **S18 Pop-out:** **Pop out** opens a `WebviewWindow` labelled `popout` loading `index.html?spike-popout=<streamer>`.
  That page connects as `v2`, watches, and logs `first_frame` with a shared epoch (`Date.now()`). The main window
  unsubscribes on the popout's `first_frame` event (Tauri `emit`) and logs `handover` with the last-frame and
  first-frame times. The blip is the gap.
- **S20 Capture check:** **Black-frame watch** samples the local preview track every 500 ms (64×36 canvas): average
  luma <8 or an unchanged hash for 3 s logs `{test:'S20',event:'black_or_frozen'}`.

`App.svelte`: when `import.meta.env.VITE_SPIKE === '1'`, import the panel lazily
(`const SpikePanel = (await import('./spike/SpikePanel.svelte')).default`). Toggle it on `Ctrl+Shift+F12`. Handle
`?spike-popout=` by rendering only the pop-out viewer.

`testpattern.html`: a standalone page. It shows a black screen that flashes white for 100 ms every 2 s, and plays a
100 ms 1 kHz beep through Web Audio at the same `performance.now()` tick. The owner opens it in Edge or Chrome and
shares that window for S9.

- [ ] **Step 5: Check and build**

Run: `cd client/ui && pnpm test && pnpm check && VITE_SPIKE=1 pnpm build`
Expected: PASS. The normal `pnpm build` (no `VITE_SPIKE`) must not include `livekit-client`. Check with
`grep -c livekit dist/assets/*.js` (0 without the flag) to confirm the panel is tree-shaken.

- [ ] **Step 6: Commit**

```bash
git add client/ui
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

Then:
- In `env:`, bump `LIVEKIT_VERSION: 1.13.10` (prod runs it since 2026-10-15) and add
  `VITE_SPIKE: ${{ inputs.spike && '1' || '' }}`.
- Change the build step to
  `pnpm dlx @tauri-apps/cli@^2 build ${{ inputs.spike && '--features spike' || '' }}`.
- Add a test step before assembling:
  `- if: inputs.spike` / `run: cargo test -p pulse-client --features spike spike:: -- --include-ignored`. If the
  loopback test can't activate audio on the runner, drop `--include-ignored` (the owner runs it locally).

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
`windows`/`wasapi` paths from Tasks 3 and 4) here, one commit per fix.

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
   - ask Claude to run `just spike-server on` (and `just spike-codecs on|off` per S1);
   - join voice on both PCs;
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
   - **"File a bug report now, titled `spike S<n>`"** at the end. Reports carry the log tail, and one per test keeps
     each under 256 KB.
4. **What to say in the report text:** the game, display mode (borderless/exclusive), anything seen by eye (stutter,
   black, echo).
5. **When done:** ask Claude to run `just spike-server off` and `just spike-codecs off`.

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
- revert prod: `just spike-server off`, `just spike-codecs off`;
- ask the owner to approve the updated spec;
- write the build-phase plans (server, streamer, viewer, adaptation, failure handling) against the measured numbers.
