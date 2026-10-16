# Optimisation Run (Step 1) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make Pulse more robust, lower-latency and lighter, per step 1 of the optimisation design, measured against the 2026-10-15 Windows baseline.

**Architecture:** 35 tasks in six phases, landing on one branch:
1. Server robustness.
2. Voice-pipeline foundation (cpal upgrade + telemetry).
3. Mouth-to-ear audio path.
4. Network and startup.
5. Join and channel switch.
6. Footprint.

Phases are ordered so that tasks rewriting shared files (`voice/devices.rs`, `voice/processor.rs`, `voice/mod.rs`, `commands.rs`, `lib.rs`, `TextChannel.svelte`) land in a fixed order. Each task names the earlier tasks it builds on. Nothing changes how the UI looks.

**Tech Stack:**
- Server: Rust (axum, sqlx/SQLite).
- Client: Tauri 2 with a Rust core (`pulse-client`: cpal, WebRTC APM, RNNoise/DeepFilterNet3, LiveKit Rust SDK 0.9.3) and a Svelte 5 UI (vitest).
- Infra: Docker Compose (Caddy, LiveKit 1.13.10).

**Spec:** `docs/superpowers/specs/2026-10-13-optimisation-design.md` (Step 1, items 1–16). Research: `docs/superpowers/research/opt-{voice,client,server,journeys}.md`.

## Global Constraints

- Work only on branch `optimisation-run`, in worktree `~/Projects/pulse-app-optimisation`. Never commit to or push `main`. Merging to main needs the owner's OK.
- The repo is PUBLIC: never commit the server IP, secrets, email addresses or people's names (code, comments, docs, commit messages).
- Prod is never touched by a task. Deploying the server (S1/S3/S4, the L1 Caddyfile) is a separate step that needs the owner's OK (Task 35).
- "Every change has a unit or integration test where behaviour changes … plus before/after numbers from step 0's logging." (spec, Testing)
- "Nothing here changes the UI's look." (spec, Step 1). The one accepted visible difference is F8: non-Latin text falls back to system-ui.
- No new runtime dependency without saying why in the task. Pinned: DeepFilterNet git `d375b2d` with tract `=0.21.4` (don't touch).
- Windows is the primary platform. Windows-only code must pass `XWIN_ACCEPT_LICENSE=1 cargo xwin clippy -p pulse-client --target x86_64-pc-windows-msvc -- -D warnings`.
- The ignored real-LiveKit suite (`voice_it`) needs `just dev-livekit`. `sudo tc netem` steps (R5) need the owner at the keyboard: ask, don't skip silently.
- Line numbers in tasks are as of `de4b2ea`. Earlier tasks shift them, so re-grep the quoted code before editing.
- Commit messages end with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

## Review Focus

These are the inputs and conditions a person is most likely to hit that the spec doesn't spell out. Each now has a test in its owning task:

1. **Pulse starts before Windows' network is up** (autostart at login): startup should wait it out (10 s grace) instead of saying "couldn't restore your session". → Task 13 (L5), `startup_survives_a_late_network`.
2. **Switching channels while the old room is reconnecting, or a switch whose join fails:** the UI should end in a clean Disconnected state, and the old room must stop getting our mic. → Task 21 (L4a), `a_failed_switch_ends_disconnected_and_the_old_room_stays_left` + `a_detached_room_gets_no_more_frames`.
3. **A Bluetooth headset reopening at the moment LiveKit gives up:** the rejoin should wait for the old device to close instead of failing every attempt. → Task 17 (R2), Step 4b.
4. **Server restarts while LiveKit is slow or down:** the server should boot within ~2 s, presence should come back once LiveKit answers, and mute/deafen should be re-declared. → Task 4 (S4) hanging/late-LiveKit tests; Task 15 (R6).
5. **Hover prefetch, then sign out and in as someone else:** the click must never join with the old account's token. → Task 16 (L3), `signing_out_forgets_prefetched_tokens`.

Further covered edge cases:
- outbox retry after a delete (S3);
- resume from sleep during backoff (R1);
- 24-bit devices (R3);
- meters after a page reload (F6);
- trimmed history vs. unread counts and in-flight fetches (F5).

## Task map (execution order)

| # | ID | Task | Phase |
|---|----|------|-------|
| 1 | S1 | Server stops cleanly on SIGTERM/SIGINT | 1 Server |
| 2 | S2 | One transaction per handler, `synchronous=NORMAL` | 1 Server |
| 3 | S3 | Idempotent send (author, nonce) | 1 Server |
| 4 | S4 | Rebuild voice presence from LiveKit at startup | 1 Server |
| 5 | R3 | cpal 0.18.2 + MMCSS real-time audio threads | 2 Foundation |
| 6 | R4 | NS attenuation + gate-open % on the path line | 2 Foundation |
| 7 | R5 | NetEq "buffer": fix deltas, measure real lag | 2 Foundation |
| 8 | L6 | Gate decides on the undelayed signal | 3 Mouth-to-ear |
| 9 | L7b | Drift-compensated playout, ~20 ms queue | 3 Mouth-to-ear |
| 10 | L8 | Mic frames straight from the processor thread | 3 Mouth-to-ear |
| 11 | L7a | Speaker callback never blocks on network locks | 3 Mouth-to-ear |
| 12 | L1 | Warm HTTPS connection | 4 Network/startup |
| 13 | L5 | Startup: gateway with stored session, no `/me` | 4 Network/startup |
| 14 | R1 | Dead gateway noticed in seconds; probe on focus/online | 4 Network/startup |
| 15 | R6 | Re-declare mute/deafen after every Ready | 4 Network/startup |
| 16 | L3 | Prefetch voice token on hover | 4 Network/startup |
| 17 | R2 | Voice rejoins by itself after LiveKit gives up | 5 Join/switch |
| 18 | L2a | Open devices while the room connects | 5 Join/switch |
| 19 | L2b | Keep LiveKit's runtime alive between calls | 5 Join/switch |
| 20 | L2c | "Connected" on room connect; publish right after | 5 Join/switch |
| 21 | L4a | Switch: old room closes in the background | 5 Join/switch |
| 22 | L4b | Switch: keep pipeline + noise model | 5 Join/switch |
| 23 | F3 | Store holds reducer output raw | 6 Footprint |
| 24 | F4 | Cache rendered markdown per message | 6 Footprint |
| 25 | F5 | Trim left channels to newest 100 messages | 6 Footprint |
| 26 | F10 | One typing-expiry timer | 6 Footprint |
| 27 | F6 | Voice levels only while a meter is shown | 6 Footprint |
| 28 | F11 | Skip the denoiser while muted | 6 Footprint |
| 29 | F2 | WebView2 low memory target when minimised | 6 Footprint |
| 30 | F7 | Measure libwebrtc log volume; filter if chatty | 6 Footprint |
| 31 | F8 | Latin-only font subsets | 6 Footprint |
| 32 | F9 | UI sounds shipped once | 6 Footprint |
| 33 | F12 | tokio-tungstenite 0.27 → 0.29 | 6 Footprint |
| 34 | F1 | Release profile (LTO, 1 CGU, strip) | 6 Footprint |
| 35 | X1 | Before/after numbers, Windows build, test protocol | Wrap-up |

Task headings below read `### Task N (ID): …`. Tasks refer to each other by ID (S/R/L/F). Older drafts used other names for some tasks: "B9" = L4a/L4b, "B10" = L5, "C15" = F6, "C16" = F11, "H1" = L6.

## Cross-phase coordination (read before the task you're on)

- **Task 5 (cpal 0.18) changes the device API.** Every later task touching `voice/devices.rs` must use `devices::device_name()`, `cpal::Error`, `u32` sample rates and configs passed by value. The device code snippets in L2a/L7a/L4b were written against 0.15, so port them.
- **`MicChain::process`:** R4 adds `ns_in`/`ns_out`/`judged` to `BlockOut`. L6 then reworks the gate and must keep those three (and `in_level` measured before any early return). F11 later adds the muted early return after them.
- **`voice/mod.rs` join/leave:** R2 adds `epoch`/`last_join`/`join_inner(.., Some(epoch))`. L2a–L4b keep them; their Coordination lines spell out how.
- **`commands.rs` / `lib.rs`:** L5 starts the gateway from `setup` via `Core::activate`. R1 adds `.on_window_event` focus probing, R6 adds the `AfterReady` hook in `Core::activate`, and F2 adds a minimise/blur handler. Merge the window-event handlers into one closure.
- **Path-line format:** R5 changes `netstats::summarize` to take `&Totals`. Later fields (`adjust`, `mix_busy`, `token_prefetched`, `pipeline_reused`) are separate tracing fields, so the exact-string tests in `netstats.rs`/`stats.rs` don't need touching again.
- **Real-time threads and WebRTC:** after L8, `pulse-mic` calls into WebRTC capture (a C++ mutex shared with the encoder). R3 raises `pulse-mic` to MMCSS. If glitches appear in the Windows test, this pairing is the first suspect.
- **Migrations:** S3 adds `0004_message_nonce.sql`. Renumber if anything lands a migration first.

---

## Phase conventions (from each section's drafter)

### A (server/deploy): clean stops, SQLite transactions, idempotent sends, voice presence after restart

Spec items A1, A5, A6, A2 (`docs/superpowers/specs/2026-10-13-optimisation-design.md`, Step 1 §A).
Order: S1 → S2 → S3 → S4 (S3 and S4 build on S2's `db::begin_write`; S4's boot test reuses S1's
binary harness). All tests use the existing in-process harness (`server/src/testing.rs`, re-exported by
`server/tests/common/mod.rs`) except where the real binary is needed (signals, boot). Nothing here
touches prod.

### B (client robustness + telemetry): dead links in seconds, voice comes back, real-time audio threads, numbers for NS/gate and NetEq

Spec items A3, A4, A7 (`docs/superpowers/specs/2026-10-13-optimisation-design.md`, Step 1 §A), plus two
items that came out of the 2026-10-15 Windows baseline (R4: NS attenuation and gate-open share on the
`voice: path` line; R5: the NetEq "buffer" question).
Assembled order: R3, R4, R5, then the latency-audio tasks, then R1 → R6, then R2 before the latency
join/switch tasks. R3 goes first because the cpal upgrade touches `devices.rs` code that the playout
section rewrites. R4 and R5 are telemetry and should be in before the Windows test. R6 builds on R1's
gateway changes. R2 rewrites `join`/`leave` before the join/switch latency work does. Paths are relative to the repo root
(`~/Projects/pulse-app-optimisation`, branch `optimisation-run`). Rust tests live in the `pulse-client`
crate (`client/src-tauri`). All `cargo` commands run from the repo root.

---

### C. Latency (spec B8-B11, plus the "output callback never blocks" part of A7)

Tasks are listed in **execution order**, not ID order (later code snippets assume the earlier tasks are in):
L1 → L5 → L3 → L6 → L7b → L8 → L7a → L2a → L2b → L2c → L4a → L4b.
All paths are relative to the repo root (`pulse-app-optimisation`, branch `optimisation-run`). Line numbers are
from HEAD `de4b2ea`. Rust tests run from the repo root; the real-LiveKit suite needs `just dev-livekit` first.

Baseline log lines to compare against (step 0, `448da41`):
- `voice: mic published token_ms devices_ms room_connect_ms publish_ms` and `voice: joined elapsed_ms`;
- `voice: path line=… send_ms=… ours="… recv a/b/c ms underrun N ms …"`;
- `startup phase=… since_start_ms=…` (`restore_begin`, `restore_end`, `gateway_open`, `ready`, `ui_painted`).

---

### D. Footprint (spec C12–C16)

Scope: release profile, WebView2 memory, UI work per event, in-call IPC, small items. Nothing here may change how the
UI looks. Line numbers are as of `optimisation-run` @ `de4b2ea`: re-grep before editing, because other sections touch
the same files (`lib.rs`, `commands.rs`, `voice/mod.rs`, `TextChannel.svelte`).

Verification status: everything below was tried before writing it down. The Rust code compiled and its tests passed in
a scratch copy of the tree (`cargo test -p pulse-client --lib`, plus `cargo xwin clippy -p pulse-client --target
x86_64-pc-windows-msvc -- -D warnings` for the Windows-only code). The UI code passed `vitest`, `svelte-check` and
`vite build` in a scratch copy. The failing-first runs were checked too (current store, current fonts, no priming).

UI tests use **vitest** (`client/ui`, `pnpm test`; jsdom per file via `// @vitest-environment jsdom`), next to the
code as `*.test.ts`. There are no component tests yet. F3 adds the resolve condition they need.

Common commands (from the repo root):
- Rust unit tests: `cargo test -p pulse-client --lib -- <filter>`
- Windows cross-check: `XWIN_ACCEPT_LICENSE=1 cargo xwin clippy -p pulse-client --target x86_64-pc-windows-msvc -- -D warnings`
- UI: `cd client/ui && pnpm test && pnpm check`

---

---

## Phase 1: Server robustness

### Task 1 (S1): Stop cleanly on SIGTERM and SIGINT (deploys close sockets with 1012)

**Why:** opt-server.md H1: `server/src/lib.rs:59` waits only on `ctrl_c()` (SIGINT); the binary is PID 1
in its container (`deploy/server.Dockerfile:27` exec-form `ENTRYPOINT`) and `deploy/compose.yml:11-30`
sets no `stop_signal`, so `docker stop`'s SIGTERM is ignored, every deploy waits 10 s and SIGKILLs, and
`hub.close_all(1012)` (`lib.rs:61`) never runs. Fix both ends: handle SIGTERM in code (works for any
supervisor, and for a bare `docker run`) and set `stop_signal: SIGINT` in compose (belt and braces).

**Files:**
- Modify: `server/src/lib.rs:58-59` (graceful-shutdown future) and append `shutdown_signal()` after `serve` (after line 66)
- Modify: `server/tests/common/mod.rs` (append a real-binary harness: `ServerProcess`, `spawn_binary`, `binary_invite`)
- Modify: `deploy/compose.yml:14` (add `stop_signal` + `stop_grace_period` under `pulse-server`)
- Test: `server/tests/shutdown.rs` (new)

**Interfaces:**
- Consumes: `pulse_server::serve`, `hub.close_all(1012)` (unchanged), the `pulse-app-server` binary (`create-invite`, `serve`).
- Produces (test harness, used by S4): in `server/tests/common/mod.rs`
  - `pub struct ServerProcess { pub child: std::process::Child, pub addr: std::net::SocketAddr, pub db_url: String, _dir: tempfile::TempDir }` with `url(&self, path: &str) -> String`, `ws_url(&self, path: &str) -> String`; killed on drop.
  - `pub async fn spawn_binary() -> ServerProcess` (fresh DB, free port, `PULSE_LIVEKIT_API_URL=http://127.0.0.1:1` so LiveKit is "down"; returns once `/health` answers).
  - `pub fn binary_invite(p: &ServerProcess) -> String`.
- Produces (server): private `async fn shutdown_signal()` in `lib.rs`.

- [ ] **Step 1: Write the failing test**

Append to `server/tests/common/mod.rs` (it already has `#![allow(dead_code)]` and `pub use pulse_server::testing::*;`):

```rust
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// The real `pulse-app-server` binary on a free port with a fresh database, for what the
/// in-process harness can't cover (signals, startup). Killed on drop.
pub struct ServerProcess {
    pub child: std::process::Child,
    pub addr: std::net::SocketAddr,
    pub db_url: String,
    _dir: tempfile::TempDir,
}

impl ServerProcess {
    pub fn url(&self, path: &str) -> String {
        format!("http://{}{}", self.addr, path)
    }
    pub fn ws_url(&self, path: &str) -> String {
        format!("ws://{}{}", self.addr, path)
    }
}

impl Drop for ServerProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn binary(db_url: &str, bind: std::net::SocketAddr) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_pulse-app-server"));
    c.env("PULSE_DB_URL", db_url)
        .env("PULSE_BIND", bind.to_string())
        .env("PULSE_LIVEKIT_KEY", "devkey")
        .env("PULSE_LIVEKIT_SECRET", "secret-secret-secret-secret-secret")
        // Nothing listens on port 1: LiveKit is "down" (ignored until S4 reads it).
        .env("PULSE_LIVEKIT_API_URL", "http://127.0.0.1:1")
        .env("RUST_LOG", "warn")
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    c
}

pub async fn spawn_binary() -> ServerProcess {
    let dir = tempfile::tempdir().unwrap();
    let db_url = format!("sqlite://{}", dir.path().join("pulse.db").display());
    // Bind :0 to find a free port, then release it for the child.
    let addr = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap();
    let child = binary(&db_url, addr).arg("serve").spawn().unwrap();
    let http = reqwest::Client::new();
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if let Ok(r) = http.get(format!("http://{addr}/health")).send().await
            && r.status() == 200
        {
            break;
        }
        assert!(Instant::now() < deadline, "server didn't come up in 15 s");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    ServerProcess {
        child,
        addr,
        db_url,
        _dir: dir,
    }
}

/// A single-use invite from the binary's own `create-invite`, against the same database.
pub fn binary_invite(p: &ServerProcess) -> String {
    let out = binary(&p.db_url, p.addr)
        .arg("create-invite")
        .stdout(Stdio::piped())
        .output()
        .unwrap();
    assert!(out.status.success(), "create-invite failed");
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}
```

Create `server/tests/shutdown.rs`:

```rust
//! Stopping the server: `docker stop` sends SIGTERM, Ctrl-C sends SIGINT. Both must close every
//! gateway socket with 1012 ("server restarting") and exit 0 promptly, not wait for a SIGKILL.
#![cfg(unix)]

mod common;

use std::time::{Duration, Instant};

use common::*;
use futures::{SinkExt, StreamExt};
use pulse_protocol::gateway::ClientFrame;
use pulse_protocol::rest::SessionResponse;
use serde_json::json;
use tokio_tungstenite::tungstenite::Message as Ws;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn connected_socket(p: &ServerProcess) -> Socket {
    let s: SessionResponse = reqwest::Client::new()
        .post(p.url("/auth/register"))
        .json(&json!({"invite_code": binary_invite(p), "username": "alex", "password": "hunter2hunter2"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let mut ws = tokio_tungstenite::connect_async(p.ws_url("/gateway"))
        .await
        .unwrap()
        .0;
    ws.send(Ws::text(
        serde_json::to_string(&ClientFrame::Hello {
            token: s.token,
            client_version: pulse_protocol::PROTOCOL_VERSION,
        })
        .unwrap(),
    ))
    .await
    .unwrap();
    loop {
        if let Some(Ok(Ws::Text(t))) = ws.next().await
            && t.contains("\"Ready\"")
        {
            return ws;
        }
    }
}

async fn stops_cleanly_on(signal: &str) {
    let mut p = spawn_binary().await;
    let mut ws = connected_socket(&p).await;
    let sent_at = Instant::now();
    let killed = std::process::Command::new("kill")
        .args([signal, &p.child.id().to_string()])
        .status()
        .unwrap();
    assert!(killed.success());

    let code = tokio::time::timeout(Duration::from_secs(3), async {
        while let Some(m) = ws.next().await {
            match m {
                Ok(Ws::Close(f)) => return f.map(|f| f.code),
                Ok(_) => continue,
                Err(_) => return None,
            }
        }
        None
    })
    .await
    .ok()
    .flatten();
    assert_eq!(
        code,
        Some(CloseCode::Restart),
        "{signal}: clients must be told the server is restarting"
    );

    let status = loop {
        if let Some(s) = p.child.try_wait().unwrap() {
            break s;
        }
        assert!(
            sent_at.elapsed() < Duration::from_secs(5),
            "{signal}: still running 5 s after the signal"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    assert!(status.success(), "{signal}: exit status {status:?}");
}

#[tokio::test]
async fn sigterm_closes_sockets_with_1012_and_exits_0() {
    stops_cleanly_on("-TERM").await;
}

#[tokio::test]
async fn sigint_closes_sockets_with_1012_and_exits_0() {
    stops_cleanly_on("-INT").await;
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p pulse-server --test shutdown`
Expected: `sigint_closes_sockets_with_1012_and_exits_0` PASSES (control: today's behaviour), and
`sigterm_closes_sockets_with_1012_and_exits_0` FAILS with
`assertion `left == right` failed: -TERM: clients must be told the server is restarting` /
`left: None` / `right: Some(Restart)` (outside a container SIGTERM's default action kills the process,
so the socket just drops).

- [ ] **Step 3: Write minimal implementation**

`server/src/lib.rs`, in `serve`, replace line 59 (`let _ = tokio::signal::ctrl_c().await;`) so the
graceful-shutdown future reads:

```rust
    .with_graceful_shutdown(async move {
        shutdown_signal().await;
        // Upgraded sockets aren't covered by graceful shutdown: tell clients to reconnect.
        hub.close_all(1012);
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    })
```

and append after `serve` (after line 66):

```rust
/// Ctrl-C (SIGINT) or SIGTERM, which is what `docker stop` sends. As PID 1 in its container the
/// server gets no default action for a signal it doesn't handle, so without this SIGTERM was
/// ignored: Docker waited 10 s, SIGKILLed it, and clients never got the 1012 close.
async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let mut term = signal(SignalKind::terminate()).expect("install SIGTERM handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = term.recv() => {}
        }
    }
    #[cfg(not(unix))]
    let _ = tokio::signal::ctrl_c().await;
}
```

`deploy/compose.yml`, under `pulse-server`, after line 14 (`restart: unless-stopped`):

```yaml
    # The server handles SIGTERM too; SIGINT is what it has always handled (belt and braces for
    # an older image). It closes sockets with 1012 and exits in ~0.2 s, so 5 s is plenty.
    stop_signal: SIGINT
    stop_grace_period: 5s
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p pulse-server --test shutdown` (both pass), then `cargo test -p pulse-server` (no
regressions) and `just deploy-check` (compose still validates; local Docker only).

Local container check (does not touch prod; builds the image locally):

```bash
docker build -f deploy/server.Dockerfile -t pulse-server:stop-test .
docker run -d --name pulse-stop-test -e PULSE_LIVEKIT_KEY=k -e PULSE_LIVEKIT_SECRET=s pulse-server:stop-test
sleep 2 && time docker stop pulse-stop-test   # expect well under 1 s (was ~10 s); exit code 0:
docker inspect -f '{{.State.ExitCode}}' pulse-stop-test   # expect 0 (137 = SIGKILLed)
docker rm pulse-stop-test
```

- [ ] **Step 5: Commit**

```bash
git add server/src/lib.rs server/tests/common/mod.rs server/tests/shutdown.rs deploy/compose.yml
git commit -m "fix(server): stop on SIGTERM so deploys close sockets with 1012 instead of a 10 s SIGKILL"
```

---

### Task 2 (S2): One transaction per handler (`BEGIN IMMEDIATE`) and `synchronous=NORMAL`

**Why:** opt-server.md M2 + M3 (journeys step 4, opt-journeys.md:297): a send is 3-5 separate
autocommits, each fsync'd under sqlx's default `synchronous=FULL` (`db.rs:9-13` sets only WAL, FKs,
busy timeout), and not atomic (a crash between the INSERT at `messages/routes.rs:193` and
`mentions::store` at `:202` loses mentions). Handlers that open a deferred `s.db.begin()` and **read
before writing** fail at once with SQLITE_BUSY (busy_timeout can't help a stale read snapshot) when
another write commits in between: a spurious 500.

Concretely, the handlers that read then write inside a transaction today (need `BEGIN IMMEDIATE`):
- `create_dm` (`servers/routes.rs:357-366`: SELECT existing DM, then INSERT channel/members; also two
  concurrent calls for the same pair can race into a 500);
- `create_channel` (`servers/routes.rs:254-261`: `SELECT MAX(position)`, then INSERT).

Handlers that write in several autocommits today (fold into one IMMEDIATE transaction):
- `messages::send` (`messages/routes.rs:193-209`: INSERT, mentions, `dm_hidden` DELETE, read-point upsert);
- `servers::join` (`servers/routes.rs:186-206`: INSERT member, SELECT channels, `init_point` per channel on a bare connection);
- `dms::remove_member` (`dms/routes.rs:188-240`: four DELETEs, then possibly DELETE the channel).

Already write-first, switched to `begin_write` only for one rule everywhere: `register`
(`auth/routes.rs:99`), `create_server` (`servers/routes.rs:110`), `add_members` (`dms/routes.rs:130`).
Single-statement writes stay autocommit (one statement is one transaction): `edit`, `remove`,
`mark_read`, `rename`, `set_mute`, `clear_mute`, `close`, `system::post`, sessions, voice webhooks,
reports.

**Files:**
- Modify: `server/src/db.rs:5-19` (imports, `.synchronous(Normal)`, new `begin_write`)
- Modify: `server/src/reads.rs:27-45` (`advance` takes a connection)
- Modify: `server/src/messages/mentions.rs:7,88-97` (`store` takes a connection)
- Modify: `server/src/messages/routes.rs:193-209` (`send` in one transaction; `dm_hidden` only for private channels)
- Modify: `server/src/dms/routes.rs:51` (`mark_read`), `:128-130` (`add_members`), `:187-250` (`remove_member`)
- Modify: `server/src/servers/routes.rs:110` (`create_server`), `:185-206` (`join`), `:254` (`create_channel`), `:357-385` (`create_dm`)
- Modify: `server/src/auth/routes.rs:99` (`register_with`)
- Test: `server/tests/sqlite_tx.rs` (new)

**Interfaces:**
- Produces: `pub async fn crate::db::begin_write(db: &SqlitePool) -> sqlx::Result<sqlx::Transaction<'static, sqlx::Sqlite>>` (used by S3, S4).
- Changes: `pub async fn reads::advance(c: &mut SqliteConnection, user: UserId, channel: ChannelId, message: MessageId) -> AppResult<bool>`;
  `pub async fn mentions::store(c: &mut SqliteConnection, message: MessageId, users: &[UserId]) -> AppResult<()>`
  (same idiom as the existing `reads::init_point(c: &mut SqliteConnection, ..)`; pass `&mut tx` or `&mut conn`).

- [ ] **Step 1: Write the failing test**

Create `server/tests/sqlite_tx.rs`:

```rust
//! SQLite discipline: WAL with synchronous=NORMAL, and read-then-write handlers that survive
//! concurrent writers (BEGIN IMMEDIATE) instead of failing with "database is locked" 500s.

mod common;

use std::collections::HashSet;

use common::*;
use pulse_protocol::rest::Channel;
use serde_json::json;

#[tokio::test]
async fn pool_uses_wal_with_synchronous_normal() {
    let app = spawn().await;
    let mode: String = sqlx::query_scalar("PRAGMA journal_mode")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(mode, "wal");
    let sync: i64 = sqlx::query_scalar("PRAGMA synchronous")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(sync, 1, "synchronous=NORMAL (FULL is 2)");
}

/// Two people double-clicking "message" at once: every call answers 200 with the same DM.
#[tokio::test]
async fn concurrent_create_dm_for_one_pair_all_succeed_with_one_channel() {
    let app = spawn().await;
    let (_, a) = register(&app, "alex").await;
    for round in 0..5 {
        let (b_id, _) = register(&app, &format!("sam{round}")).await;
        let calls = (0..16).map(|_| post_json(&app, &a, "/dms", json!({ "user_ids": [b_id] })));
        let mut ids = HashSet::new();
        for r in futures::future::join_all(calls).await {
            let status = r.status();
            let body = r.text().await.unwrap();
            assert_eq!(status, 200, "round {round}: {body}");
            let ch: Channel = serde_json::from_str(&body).unwrap();
            ids.insert(ch.id);
        }
        assert_eq!(ids.len(), 1, "round {round}: one DM per pair");
    }
}

#[tokio::test]
async fn concurrent_create_channel_all_succeed_with_distinct_positions() {
    let app = spawn().await;
    let (_, a) = register(&app, "alex").await;
    let s = create_server(&app, &a, "Main").await;
    let path = format!("/servers/{}/channels", s.id);
    let calls = (0..16).map(|i| {
        post_json(&app, &a, &path, json!({ "kind": "text", "name": format!("c{i}") }))
    });
    let mut positions = HashSet::new();
    for r in futures::future::join_all(calls).await {
        let status = r.status();
        let body = r.text().await.unwrap();
        assert_eq!(status, 200, "{body}");
        let ch: Channel = serde_json::from_str(&body).unwrap();
        positions.insert(ch.position);
    }
    assert_eq!(positions.len(), 16, "no two channels share a position");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p pulse-server --test sqlite_tx`
Expected: `pool_uses_wal_with_synchronous_normal` FAILS deterministically (`left: 2, right: 1`). The two
concurrency tests are races: they normally fail with `status 500` and a body mentioning
`database is locked` (SQLITE_BUSY_SNAPSHOT). If one happens to pass, rerun once; the PRAGMA test is the
deterministic gate.

- [ ] **Step 3: Write minimal implementation**

`server/src/db.rs`, replace lines 5-19 (import + `connect`) and add `begin_write` below it:

```rust
use sqlx::sqlite::{
    SqliteConnectOptions, SqliteJournalMode, SqlitePool, SqlitePoolOptions, SqliteSynchronous,
};
use sqlx::{Sqlite, Transaction};

/// Open (creating if needed) the database in WAL mode with FKs on, and run migrations.
pub async fn connect(url: &str) -> anyhow::Result<SqlitePool> {
    let opts = SqliteConnectOptions::from_str(url)?
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        // Safe from corruption in WAL; an OS crash or power cut (not a process crash) can roll back
        // the last commits. One fsync per checkpoint instead of one per commit.
        .synchronous(SqliteSynchronous::Normal)
        .foreign_keys(true)
        .busy_timeout(std::time::Duration::from_secs(5));
    let pool = SqlitePoolOptions::new()
        .max_connections(8)
        .connect_with(opts)
        .await?;
    sqlx::migrate!("./migrations").run(&pool).await?;
    Ok(pool)
}

/// Start a transaction that will write. `BEGIN IMMEDIATE` takes the write lock up front, so a
/// handler that reads and then writes queues behind other writers (busy_timeout) instead of
/// failing at once with SQLITE_BUSY when a write commits between its read and its write.
pub async fn begin_write(db: &SqlitePool) -> sqlx::Result<Transaction<'static, Sqlite>> {
    db.begin_with("BEGIN IMMEDIATE").await
}
```

`server/src/reads.rs`, replace `advance` (lines 27-45):

```rust
/// Move the point forward to `message` (never backwards). True if it moved.
pub async fn advance(
    c: &mut SqliteConnection,
    user: UserId,
    channel: ChannelId,
    message: MessageId,
) -> AppResult<bool> {
    let r = sqlx::query(
        "INSERT INTO read_states (user_id, channel_id, last_read_message_id) VALUES (?, ?, ?)
         ON CONFLICT (user_id, channel_id) DO UPDATE SET last_read_message_id = excluded.last_read_message_id
         WHERE read_states.last_read_message_id IS NULL OR excluded.last_read_message_id > read_states.last_read_message_id",
    )
    .bind(user.to_string())
    .bind(channel.to_string())
    .bind(message.to_string())
    .execute(&mut *c)
    .await?;
    Ok(r.rows_affected() > 0)
}
```

`server/src/messages/mentions.rs`: line 7 becomes `use sqlx::{SqliteConnection, SqlitePool};`, and
replace `store` (lines 88-97):

```rust
pub async fn store(c: &mut SqliteConnection, message: MessageId, users: &[UserId]) -> AppResult<()> {
    for u in users {
        sqlx::query("INSERT OR IGNORE INTO mentions (message_id, user_id) VALUES (?, ?)")
            .bind(message.to_string())
            .bind(u.to_string())
            .execute(&mut *c)
            .await?;
    }
    Ok(())
}
```

`server/src/messages/routes.rs`, in `send`, replace lines 193-209 (INSERT through `let moved = ...`):

```rust
    // One transaction: the message, its mentions and your read point land together or not at all.
    let mut tx = crate::db::begin_write(&s.db).await?;
    sqlx::query("INSERT INTO messages (id, channel_id, author_id, kind, content, reply_to_id, created_at) VALUES (?, ?, ?, 'normal', ?, ?, ?)")
        .bind(msg.id.to_string())
        .bind(id.to_string())
        .bind(me.to_string())
        .bind(&msg.content)
        .bind(msg.reply_to_id.map(|r| r.to_string()))
        .bind(&msg.created_at)
        .execute(&mut *tx)
        .await?;
    mentions::store(&mut tx, msg.id, &mentioned).await?;
    if ch.kind.is_private() {
        // A new message reopens closed conversations (clients un-hide on MessageCreated).
        // Server channels can't be closed, so they skip this write entirely.
        sqlx::query("DELETE FROM dm_hidden WHERE channel_id = ?")
            .bind(id.to_string())
            .execute(&mut *tx)
            .await?;
    }
    // Sending marks your own read point (you've obviously seen everything up to here).
    let moved = crate::reads::advance(&mut tx, me, id, msg.id).await?;
    tx.commit().await?;
```

`server/src/dms/routes.rs`, `mark_read` line 51 becomes:

```rust
    let moved = {
        let mut c = s.db.acquire().await?;
        crate::reads::advance(&mut c, me, id, req.message_id).await?
    };
    if moved {
```

`add_members`, lines 128-130 become:

```rust
    // Write first, then count, under the write lock: two adds racing past the check above
    // can't overfill the group.
    let mut tx = crate::db::begin_write(&s.db).await?;
```

`remove_member`, replace lines 187-250 (from `group_for(&s, me, id).await?;` through the
`GroupMembersChanged` publish; the system-line code from line 251 stays):

```rust
    group_for(&s, me, id).await?;
    // One transaction: a crash or a concurrent add can't leave a half-removed member or an
    // empty group behind.
    let mut tx = crate::db::begin_write(&s.db).await?;
    let removed = sqlx::query("DELETE FROM channel_members WHERE channel_id = ? AND user_id = ?")
        .bind(id.to_string())
        .bind(user.to_string())
        .execute(&mut *tx)
        .await?
        .rows_affected();
    if removed == 0 {
        return Err(AppError::NotFound); // dropping `tx` rolls back
    }
    for t in ["read_states", "dm_hidden"] {
        sqlx::query(&format!(
            "DELETE FROM {t} WHERE channel_id = ? AND user_id = ?"
        ))
        .bind(id.to_string())
        .bind(user.to_string())
        .execute(&mut *tx)
        .await?;
    }
    let unmuted = sqlx::query(
        "DELETE FROM notification_prefs WHERE user_id = ? AND target_kind = 'channel' AND target_id = ?",
    )
    .bind(user.to_string())
    .bind(id.to_string())
    .execute(&mut *tx)
    .await?
    .rows_affected();
    let remaining: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM channel_members WHERE channel_id = ?")
            .bind(id.to_string())
            .fetch_one(&mut *tx)
            .await?;
    if remaining == 0 {
        sqlx::query("DELETE FROM channels WHERE id = ?")
            .bind(id.to_string())
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    if unmuted > 0 {
        s.hub
            .publish(
                &s.db,
                Event::MutesChanged {
                    user_id: user,
                    mutes: mutes_of(&s.db, user).await?,
                },
            )
            .await;
    }
    s.hub
        .publish(
            &s.db,
            Event::ChannelRemoved {
                channel_id: id,
                user_id: user,
            },
        )
        .await;
    if remaining == 0 {
        return Ok(StatusCode::NO_CONTENT);
    }
    s.hub
        .publish(
            &s.db,
            Event::GroupMembersChanged {
                channel_id: id,
                user_ids: members_of(&s.db, id).await?,
            },
        )
        .await;
```

(`MutesChanged` and `ChannelRemoved` are `only_for` the removed user, so publishing after the channel
row is gone is safe: `gateway/audience.rs:30-32` returns before `load_channel`.)

`server/src/servers/routes.rs`:
- line 110 (`create_server`): `let mut tx = crate::db::begin_write(&s.db).await?;`
- line 254 (`create_channel`): `let mut tx = crate::db::begin_write(&s.db).await?;`
- `join`, replace lines 185-206 (from `let added = ...` through the end of the `init_point` loop) and keep the publish:

```rust
    // One transaction, under the write lock: a channel created concurrently is either seen here
    // (and gets a read point) or sees us as a member in create_channel.
    let mut tx = crate::db::begin_write(&s.db).await?;
    let added = sqlx::query(
        "INSERT OR IGNORE INTO server_members (server_id, user_id, joined_at) VALUES (?, ?, ?)",
    )
    .bind(id.to_string())
    .bind(me.to_string())
    .bind(now())
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if added > 0 {
        // Start at "all read": no wall of old unreads in a server you just joined.
        let text: Vec<String> =
            sqlx::query_scalar("SELECT id FROM channels WHERE server_id = ? AND kind = 'text'")
                .bind(id.to_string())
                .fetch_all(&mut *tx)
                .await?;
        for ch in text {
            crate::reads::init_point(&mut tx, me, ch.parse().map_err(anyhow::Error::from)?).await?;
        }
    }
    tx.commit().await?;
    if added > 0 {
        s.hub
            .publish(
                &s.db,
                Event::MemberJoined {
                    server_id: id,
                    member: member_of(&s.db, id, me).await?,
                },
            )
            .await;
    }
    Ok(StatusCode::NO_CONTENT)
```

- `create_dm`, line 357 becomes `let mut tx = crate::db::begin_write(&s.db).await?;`, and the
  existing-DM branch (lines 369-386) does its reopen inside that transaction:

```rust
        if let Some(id) = existing {
            let channel_id: ChannelId = id.parse().map_err(anyhow::Error::from)?;
            // Starting a DM you had closed reopens it.
            let reopened =
                sqlx::query("DELETE FROM dm_hidden WHERE user_id = ? AND channel_id = ?")
                    .bind(me.to_string())
                    .bind(&id)
                    .execute(&mut *tx)
                    .await?
                    .rows_affected();
            tx.commit().await?;
            if reopened > 0 {
```

(the rest of the branch, from `s.hub.publish(... ConversationVisibility ...)`, is unchanged).

`server/src/auth/routes.rs` line 99: `let mut tx = crate::db::begin_write(&s.db).await?;`

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p pulse-server --test sqlite_tx` (all 3 pass; run it 3 times to shake the races),
then `cargo test -p pulse-server` (existing `messages`, `dms`, `servers`, `review_fixes` suites still
pass) and `cargo clippy -p pulse-server --all-targets -- -D warnings`.

- [ ] **Step 5: Commit**

```bash
git add server/src/db.rs server/src/reads.rs server/src/messages/mentions.rs server/src/messages/routes.rs \
  server/src/dms/routes.rs server/src/servers/routes.rs server/src/auth/routes.rs server/tests/sqlite_tx.rs
git commit -m "fix(server): one BEGIN IMMEDIATE transaction per write handler, synchronous=NORMAL"
```

---

### Task 3 (S3): Idempotent send: unique (author, nonce), retries return the original

**Why:** opt-server.md M7: the client outbox retries on 502/503 (`client/src-tauri/src/api.rs:30-39`
says "a duplicate is possible"; retry at `outbox.rs:97`), but the server's `send`
(`messages/routes.rs:138`) never stores or checks `req.nonce`, so a proxy 502 after the upstream
committed (typical during a deploy) double-posts. The client already sends a stable nonce per message:
the UI makes it with `crypto.randomUUID()` (`client/ui/src/components/TextChannel.svelte:169`), and the
outbox reuses it on every automatic retry (`outbox.rs:92-96`). A manual "Retry" after a failure makes a
new nonce (`TextChannel.svelte:178-183`), which is right: that is a deliberate resend.
**Rollback note for the deploy:** after migration 0004, an older image won't start against the
migrated database (sqlx `VersionMissing`). To roll back past S3, restore the pre-deploy backup that
`just deploy` takes.

**Files:**
- Create: `server/migrations/0004_message_nonce.sql`
- Modify: `server/src/messages/routes.rs:22` (`MAX_NONCE`), `:159` (validate), and the transaction block S2 introduced in `send` (dedupe check + store nonce)
- Test: `server/tests/send_idempotency.rs` (new)

**Interfaces:**
- Consumes: `crate::db::begin_write` (S2); `SendMessageRequest.nonce: Option<String>` (`protocol/src/rest.rs:207-211`, unchanged, so no TS type regeneration).
- Produces: `messages.nonce TEXT NULL` column and partial unique index `messages_author_nonce ON messages (author_id, nonce) WHERE nonce IS NOT NULL`. `POST /channels/{id}/messages` with a nonce already used by the same author returns **200 with the original message** and publishes nothing; nonce longer than 64 bytes or empty is 400.

- [ ] **Step 1: Write the failing test**

Create `server/tests/send_idempotency.rs`:

```rust
//! The outbox retries a send with the same nonce (e.g. after a proxy 502 that hid a success):
//! the server must post it once and hand back the original.

mod common;

use std::time::Duration;

use common::*;
use pulse_protocol::gateway::{Event, ServerFrame};
use pulse_protocol::ids::{ChannelId, UserId};
use pulse_protocol::rest::Message;
use serde_json::json;

struct Setup {
    app: TestApp,
    a: String,
    b_id: UserId,
    b: String,
    g: ChannelId,
}

async fn setup() -> Setup {
    let app = spawn().await;
    let (_, a) = register(&app, "alex").await;
    let (b_id, b) = register(&app, "sam").await;
    let s = create_server(&app, &a, "Main").await;
    post_json(&app, &b, &format!("/servers/{}/join", s.id), json!({})).await;
    let g = general(&app, &a, s.id).await.id;
    Setup { app, a, b_id, b, g }
}

async fn send_nonce(
    app: &TestApp,
    token: &str,
    channel: ChannelId,
    content: &str,
    nonce: &str,
) -> reqwest::Response {
    post_json(
        app,
        token,
        &format!("/channels/{channel}/messages"),
        json!({ "content": content, "nonce": nonce }),
    )
    .await
}

async fn normal_rows(app: &TestApp, channel: ChannelId) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM messages WHERE channel_id = ? AND kind = 'normal'")
        .bind(channel.to_string())
        .fetch_one(&app.db)
        .await
        .unwrap()
}

#[tokio::test]
async fn retry_with_same_nonce_returns_original_and_posts_once() {
    let t = setup().await;
    let mut rx = t.app.hub.register(t.b_id, String::new()).rx;
    let first: Message = send_nonce(&t.app, &t.a, t.g, "hello", "n-1").await.json().await.unwrap();
    let r = send_nonce(&t.app, &t.a, t.g, "hello", "n-1").await;
    assert_eq!(r.status(), 200);
    let again: Message = r.json().await.unwrap();
    assert_eq!(again.id, first.id);
    assert_eq!(again.content, "hello");
    assert_eq!(normal_rows(&t.app, t.g).await, 1);

    // Others saw it exactly once.
    let mut created = 0;
    while let Ok(Some(f)) = tokio::time::timeout(Duration::from_millis(200), rx.recv()).await {
        if matches!(f, ServerFrame::Event(Event::MessageCreated { .. })) {
            created += 1;
        }
    }
    assert_eq!(created, 1);
}

#[tokio::test]
async fn concurrent_duplicates_post_once() {
    let t = setup().await;
    let (r1, r2) = futures::future::join(
        send_nonce(&t.app, &t.a, t.g, "hi", "n-race"),
        send_nonce(&t.app, &t.a, t.g, "hi", "n-race"),
    )
    .await;
    assert_eq!(r1.status(), 200);
    assert_eq!(r2.status(), 200);
    let (m1, m2): (Message, Message) = (r1.json().await.unwrap(), r2.json().await.unwrap());
    assert_eq!(m1.id, m2.id);
    assert_eq!(normal_rows(&t.app, t.g).await, 1);
}

#[tokio::test]
async fn same_nonce_from_different_authors_are_different_messages() {
    let t = setup().await;
    let m1: Message = send_nonce(&t.app, &t.a, t.g, "x", "same").await.json().await.unwrap();
    let m2: Message = send_nonce(&t.app, &t.b, t.g, "x", "same").await.json().await.unwrap();
    assert_ne!(m1.id, m2.id);
    assert_eq!(normal_rows(&t.app, t.g).await, 2);
}

#[tokio::test]
async fn sends_without_nonce_are_never_deduplicated() {
    let t = setup().await;
    send(&t.app, &t.a, t.g, "same text").await;
    send(&t.app, &t.a, t.g, "same text").await;
    assert_eq!(normal_rows(&t.app, t.g).await, 2);
}

/// The send landed but the client saw a 502; the user deleted the message (from another window)
/// before the outbox retried. The retry must not resurrect it as a new message.
#[tokio::test]
async fn retry_after_delete_returns_the_deleted_original() {
    let t = setup().await;
    let first: Message = send_nonce(&t.app, &t.a, t.g, "oops", "n-del").await.json().await.unwrap();
    let del = t
        .app
        .http
        .delete(t.app.url(&format!("/messages/{}", first.id)))
        .bearer_auth(&t.a)
        .send()
        .await
        .unwrap();
    assert_eq!(del.status(), 204);

    let r = send_nonce(&t.app, &t.a, t.g, "oops", "n-del").await;
    assert_eq!(r.status(), 200);
    let again: Message = r.json().await.unwrap();
    assert_eq!(again.id, first.id);
    assert!(again.deleted);
    assert_eq!(again.content, "", "a deleted message never exposes content");
    assert_eq!(normal_rows(&t.app, t.g).await, 1);
}

#[tokio::test]
async fn oversized_nonce_is_400() {
    let t = setup().await;
    let r = send_nonce(&t.app, &t.a, t.g, "x", &"n".repeat(65)).await;
    assert_eq!(r.status(), 400);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p pulse-server --test send_idempotency`
Expected: `retry_with_same_nonce_returns_original_and_posts_once` fails (`left: MessageId(..)` ≠ first
id), `concurrent_duplicates_post_once` fails (`left: 2, right: 1` rows), `oversized_nonce_is_400` fails
(`left: 200, right: 400`), and `retry_after_delete_returns_the_deleted_original` fails because the
retry posts a fresh message (`left: MessageId(..)` ≠ first id). The two "different" tests pass
(regression guards). No extra implementation is needed for the delete case. The dedupe lookup
ignores `deleted_at`, and `load` → `from_row` (`messages/routes.rs:51-75`) already blanks the content
and sets `deleted: true`. The outbox then gets the deleted original, so it marks the item sent and
nothing is re-posted.

- [ ] **Step 3: Write minimal implementation**

Create `server/migrations/0004_message_nonce.sql`:

```sql
-- Idempotent sends (spec 2026-10-13-optimisation-design, A6): the outbox retries with the same
-- client nonce, so keep at most one message per (author, nonce). Old rows and system lines have none.
ALTER TABLE messages ADD COLUMN nonce TEXT;
CREATE UNIQUE INDEX messages_author_nonce ON messages (author_id, nonce) WHERE nonce IS NOT NULL;
```

`server/src/messages/routes.rs`, after line 22 (`const MAX_LEN`):

```rust
/// Client nonces are UUIDs (36 bytes); anything much longer isn't one of ours.
const MAX_NONCE: usize = 64;
```

In `send`, after `let content = clean(&req.content)?;` (line 159):

```rust
    if req
        .nonce
        .as_ref()
        .is_some_and(|n| n.is_empty() || n.len() > MAX_NONCE)
    {
        return Err(AppError::BadRequest(format!(
            "nonce must be 1-{MAX_NONCE} bytes"
        )));
    }
```

In the transaction block from S2, between `let mut tx = crate::db::begin_write(&s.db).await?;` and the
INSERT, add the dedupe check, and store the nonce in the INSERT:

```rust
    let mut tx = crate::db::begin_write(&s.db).await?;
    // An outbox retry of a send that already landed (e.g. the proxy said 502 after we committed):
    // hand back the original instead of posting twice. Under the write lock, check-then-insert is
    // atomic; the unique index is the backstop.
    if let Some(nonce) = &req.nonce
        && let Some(existing) = sqlx::query_scalar::<_, String>(
            "SELECT id FROM messages WHERE author_id = ? AND nonce = ?",
        )
        .bind(me.to_string())
        .bind(nonce)
        .fetch_optional(&mut *tx)
        .await?
    {
        tx.rollback().await?;
        // Not re-published: the first send already fanned it out, and a second MessageCreated
        // would double-count unreads on other clients.
        return load(&s.db, parse(&existing)?)
            .await?
            .map(Json)
            .ok_or(AppError::NotFound);
    }
    sqlx::query("INSERT INTO messages (id, channel_id, author_id, kind, content, reply_to_id, created_at, nonce) VALUES (?, ?, ?, 'normal', ?, ?, ?, ?)")
        .bind(msg.id.to_string())
        .bind(id.to_string())
        .bind(me.to_string())
        .bind(&msg.content)
        .bind(msg.reply_to_id.map(|r| r.to_string()))
        .bind(&msg.created_at)
        .bind(&req.nonce)
        .execute(&mut *tx)
        .await?;
```

(`load`, `parse` and `AppError` already exist in this file: `routes.rs:44-49`, `:77-90`.)

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p pulse-server --test send_idempotency`, then `cargo test -p pulse-server` and
`cargo test -p pulse-client outbox` (client outbox tests unaffected). If the new migration is not
picked up (stale `sqlx::migrate!` expansion), `touch server/src/db.rs` and rerun.

- [ ] **Step 5: Commit**

```bash
git add server/migrations/0004_message_nonce.sql server/src/messages/routes.rs server/tests/send_idempotency.rs
git commit -m "fix(server): dedupe message sends on (author, nonce) so outbox retries never double-post" \
  -m "Rollback: migration 0004 means an older image won't boot against the migrated DB; restore the pre-deploy backup to roll back past this."
```

---

### Task 4 (S4): Rebuild voice presence from LiveKit at startup

**Why:** opt-server.md H2: `VoiceState` is in memory only (`server/src/voice/state.rs:1-3` calls the
rebuild "a follow-up"), and `voice::reconcile_on_startup` (`voice/mod.rs:9-21`, called at
`lib.rs:43`) closes every open `voice_sessions` row at zero length. LiveKit keeps calls running across
a Pulse deploy and never re-sends `participant_joined`, so after every deploy Ready says nobody is in
voice (`gateway/socket.rs:168-179`) and stats lose the sessions. Identity format is fixed by our own
token: identity = `UserId` string, room = `ChannelId` string (`voice/routes.rs:43-52`, parsed the same
way in the webhook at `:84-88`).

Design: at boot, after the DB is up and before serving, call LiveKit's RoomService (`ListRooms`, then
`ListParticipants` per room) over loopback with a 2 s budget. Success: fill `VoiceState`, keep open
session rows of people still in calls, close the rest at zero length, and open rows for people who
joined while we were down (from LiveKit's `joined_at`). Failure (LiveKit down, slow, or a `wss://` URL
this TLS-less client can't call): log a warning, fall back to today's zero-length close, start empty,
and keep retrying in the background every 5 s for a minute, publishing `VoiceJoined` for whoever it
finds so connected clients catch up.

**Files:**
- Modify: `server/Cargo.toml` (`livekit-api` feature `services`; add `livekit-protocol = "0.8.0"`, already in `Cargo.lock`; dev-dep `prost = "0.14"`)
- Modify: `server/src/config.rs:123-153` (new `livekit_api_url`)
- Modify: `server/src/testing.rs:38-49` (`test_config`) and `:23-36` (`TestApp::state()`)
- Create: `server/src/voice/restore.rs`
- Modify: `server/src/voice/mod.rs:1-2` (`pub mod restore;`) and `:6-8` (doc comment)
- Modify: `server/src/voice/state.rs:1-3` (doc) and add `VoiceState::restore` after `join` (after line 53 of the file, i.e. the end of `join`)
- Modify: `server/src/lib.rs:41-53` (`serve`: restore before serving, drop the bare `reconcile_on_startup` call)
- Modify: `deploy/compose.yml:19` (add `PULSE_LIVEKIT_API_URL`)
- Test: `server/tests/voice_restore.rs` (new)

**Interfaces:**
- Consumes: `crate::db::begin_write` (S2); `ServerProcess`/`spawn_binary` (S1, for the boot test); `livekit_api::services::room::RoomClient::{with_api_key, with_failover, with_request_timeout, list_rooms, list_participants}` (livekit-api 0.8.1).
- Produces:
  - `Config { pub livekit_api_url: String, .. }`, env `PULSE_LIVEKIT_API_URL`, default = `PULSE_LIVEKIT_URL`.
  - `pub struct voice::restore::LiveParticipant { pub channel: ChannelId, pub user: UserId, pub sid: String, pub joined_at: String }`
  - `pub async fn voice::restore::fetch(url: &str, key: &str, secret: &str, budget: Duration) -> anyhow::Result<Vec<LiveParticipant>>`
  - `pub async fn voice::restore::apply(db: &SqlitePool, voice: &VoiceState, live: &[LiveParticipant], started_at: &str) -> anyhow::Result<Vec<LiveParticipant>>` (returns those newly added to `VoiceState`)
  - `pub async fn voice::restore::sync(s: &AppState, started_at: &str, budget: Duration) -> anyhow::Result<usize>`
  - `pub async fn voice::restore::on_startup(s: &AppState, started_at: &str) -> anyhow::Result<()>`
  - `pub async fn voice::restore::retry_loop(s: AppState, started_at: String, every: Duration, tries: u32)` (spawned by `on_startup` with 5 s × 12; tests pass small values)
  - `pub fn VoiceState::restore(&self, channel: ChannelId, user: UserId, sid: &str) -> bool`
  - `pub fn TestApp::state(&self) -> AppState`

- [ ] **Step 1: Write the failing test**

Add to `server/Cargo.toml` `[dev-dependencies]` (needed by the fake LiveKit in the test):

```toml
prost = "0.14"
livekit-protocol = "0.8.0"
```

Create `server/tests/voice_restore.rs`:

```rust
//! Voice presence after a restart: rebuilt from LiveKit's room list (a fake RoomService here,
//! speaking LiveKit's Twirp protocol: POST /twirp/livekit.RoomService/<Method>, protobuf bodies).

mod common;

use std::sync::Arc;
use std::time::Duration;

use axum::body::Bytes;
use axum::routing::post;
use common::*;
use livekit_protocol as lkp;
use prost::Message as _;
use pulse_protocol::gateway::{Event, ServerFrame};
use pulse_protocol::ids::{ChannelId, UserId};
use pulse_protocol::rest::Channel;
use pulse_server::voice::restore;
use pulse_server::voice::state::Left;

const BUDGET: Duration = Duration::from_secs(2);
/// 2026-01-01T00:00:00Z
const JAN_1: i64 = 1_767_225_600;

type Rooms = Vec<(String, Vec<lkp::ParticipantInfo>)>;

/// A fake RoomService on 127.0.0.1:<free port>; returns its base URL.
async fn fake_livekit(rooms: Rooms) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    serve_fake(listener, rooms);
    format!("http://{addr}")
}

/// A RoomService that accepts the connection and never answers (overloaded or mid-restart).
async fn fake_livekit_hanging() -> String {
    let app = axum::Router::new().route(
        "/twirp/livekit.RoomService/ListRooms",
        post(|| async {
            tokio::time::sleep(Duration::from_secs(30)).await;
            Vec::<u8>::new()
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

/// Serve the fake RoomService on an already-bound listener (lets a test bring LiveKit up late).
fn serve_fake(listener: tokio::net::TcpListener, rooms: Rooms) {
    let rooms = Arc::new(rooms);
    let r = rooms.clone();
    let app = axum::Router::new()
        .route(
            "/twirp/livekit.RoomService/ListRooms",
            post(move || {
                let rooms = r.clone();
                async move {
                    lkp::ListRoomsResponse {
                        rooms: rooms
                            .iter()
                            .map(|(name, _)| lkp::Room {
                                name: name.clone(),
                                ..Default::default()
                            })
                            .collect(),
                    }
                    .encode_to_vec()
                }
            }),
        )
        .route(
            "/twirp/livekit.RoomService/ListParticipants",
            post(move |body: Bytes| {
                let rooms = rooms.clone();
                async move {
                    let req = lkp::ListParticipantsRequest::decode(body).unwrap();
                    let participants = rooms
                        .iter()
                        .find(|(name, _)| *name == req.room)
                        .map(|(_, p)| p.clone())
                        .unwrap_or_default();
                    lkp::ListParticipantsResponse { participants }.encode_to_vec()
                }
            }),
        );
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
}

fn participant(user: UserId, sid: &str) -> lkp::ParticipantInfo {
    lkp::ParticipantInfo {
        identity: user.to_string(),
        sid: sid.into(),
        state: lkp::participant_info::State::Active as i32,
        joined_at: JAN_1,
        ..Default::default()
    }
}

/// The app's state, pointed at `livekit` for API calls.
fn state_with(app: &TestApp, livekit: String) -> pulse_server::AppState {
    let mut s = app.state();
    s.cfg = Arc::new(pulse_server::config::Config {
        livekit_api_url: livekit,
        ..(*app.cfg).clone()
    });
    s
}

async fn lounge(app: &TestApp, token: &str) -> Channel {
    let s = create_server(app, token, "Main").await;
    let chans: Vec<Channel> = get_json(app, token, &format!("/servers/{}/channels", s.id)).await;
    chans
        .into_iter()
        .find(|c| c.name.as_deref() == Some("Lounge"))
        .unwrap()
}

async fn open_session(app: &TestApp, id: &str, user: UserId, channel: ChannelId) {
    sqlx::query("INSERT INTO voice_sessions (id, user_id, channel_id, joined_at) VALUES (?, ?, ?, '2026-01-01T00:00:00.000Z')")
        .bind(id)
        .bind(user.to_string())
        .bind(channel.to_string())
        .execute(&app.db)
        .await
        .unwrap();
}

async fn session(app: &TestApp, user: UserId) -> Vec<(String, String, Option<String>)> {
    sqlx::query_as("SELECT id, joined_at, left_at FROM voice_sessions WHERE user_id = ? ORDER BY joined_at")
        .bind(user.to_string())
        .fetch_all(&app.db)
        .await
        .unwrap()
}

#[tokio::test]
async fn restart_mid_call_restores_presence_and_keeps_the_session_open() {
    let app = spawn().await;
    let (a_id, a) = register(&app, "alex").await;
    let l = lounge(&app, &a).await;
    open_session(&app, "VS1", a_id, l.id).await;
    let lk = fake_livekit(vec![(l.id.to_string(), vec![participant(a_id, "PA_1")])]).await;
    let mut rx = app.hub.register(a_id, String::new()).rx;

    let n = restore::sync(&state_with(&app, lk), &pulse_server::db::now(), BUDGET)
        .await
        .unwrap();

    assert_eq!(n, 1);
    assert!(app.voice_members(l.id).contains(&a_id));
    assert_eq!(
        session(&app, a_id).await,
        vec![("VS1".into(), "2026-01-01T00:00:00.000Z".into(), None)],
        "the pre-restart session row stays open"
    );
    match rx.recv().await.unwrap() {
        ServerFrame::Event(Event::VoiceJoined { channel_id, user_id, .. }) => {
            assert_eq!((channel_id, user_id), (l.id, a_id));
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[tokio::test]
async fn left_while_down_is_closed_and_joined_while_down_is_opened() {
    let app = spawn().await;
    let (a_id, a) = register(&app, "alex").await;
    let (b_id, _) = register(&app, "sam").await;
    let l = lounge(&app, &a).await;
    open_session(&app, "VS_B", b_id, l.id).await; // sam left while the server was down
    let lk = fake_livekit(vec![(l.id.to_string(), vec![participant(a_id, "PA_1")])]).await;

    restore::sync(&state_with(&app, lk), &pulse_server::db::now(), BUDGET)
        .await
        .unwrap();

    assert_eq!(
        session(&app, b_id).await,
        vec![(
            "VS_B".into(),
            "2026-01-01T00:00:00.000Z".into(),
            Some("2026-01-01T00:00:00.000Z".into())
        )],
        "closed at zero length: we can't know when they left"
    );
    let a_rows = session(&app, a_id).await;
    assert_eq!(a_rows.len(), 1);
    assert_eq!(a_rows[0].1, "2026-01-01T00:00:00.000Z", "LiveKit's join time");
    assert_eq!(a_rows[0].2, None);
    assert!(!app.voice_members(l.id).contains(&b_id));
}

#[tokio::test]
async fn foreign_rooms_identities_and_disconnected_participants_are_ignored() {
    let app = spawn().await;
    let (a_id, a) = register(&app, "alex").await;
    let l = lounge(&app, &a).await;
    let mut gone = participant(a_id, "PA_old");
    gone.state = lkp::participant_info::State::Disconnected as i32;
    let bot = lkp::ParticipantInfo {
        identity: "egress-bot".into(),
        ..Default::default()
    };
    let lk = fake_livekit(vec![
        ("not-a-channel".into(), vec![participant(a_id, "PA_x")]),
        (l.id.to_string(), vec![gone, bot]),
    ])
    .await;

    let n = restore::sync(&state_with(&app, lk), &pulse_server::db::now(), BUDGET)
        .await
        .unwrap();

    assert_eq!(n, 0);
    assert!(app.voice_members(l.id).is_empty());
}

#[tokio::test]
async fn a_webhook_join_that_beat_the_restore_wins() {
    let app = spawn().await;
    let (a_id, a) = register(&app, "alex").await;
    let l = lounge(&app, &a).await;
    app.voice.join(l.id, a_id, "PA_new");
    let lk = fake_livekit(vec![(l.id.to_string(), vec![participant(a_id, "PA_old")])]).await;

    let n = restore::sync(&state_with(&app, lk), &pulse_server::db::now(), BUDGET)
        .await
        .unwrap();

    assert_eq!(n, 0, "already present: no second VoiceJoined");
    // The current connection is still PA_new, so a `left` for PA_old is stale.
    assert_eq!(app.voice.leave(l.id, a_id, "PA_old"), Left::Stale);
}

#[tokio::test]
async fn livekit_unreachable_is_an_error_and_changes_nothing() {
    let app = spawn().await;
    let (a_id, a) = register(&app, "alex").await;
    let l = lounge(&app, &a).await;
    open_session(&app, "VS1", a_id, l.id).await;

    let started = std::time::Instant::now();
    let r = restore::sync(
        &state_with(&app, "http://127.0.0.1:1".into()),
        &pulse_server::db::now(),
        BUDGET,
    )
    .await;

    assert!(r.is_err());
    assert!(started.elapsed() < BUDGET + Duration::from_secs(1));
    assert!(app.voice_members(l.id).is_empty());
    assert_eq!(session(&app, a_id).await[0].2, None, "rows untouched by a failed sync");
}

/// LiveKit takes the TCP connection but never answers: boot must not wait longer than its
/// budget, and must fall back to closing old sessions.
#[tokio::test]
async fn hanging_livekit_at_boot_is_bounded_and_falls_back() {
    let app = spawn().await;
    let (a_id, a) = register(&app, "alex").await;
    let l = lounge(&app, &a).await;
    open_session(&app, "VS1", a_id, l.id).await;
    let s = state_with(&app, fake_livekit_hanging().await);

    let started = std::time::Instant::now();
    restore::on_startup(&s, &pulse_server::db::now())
        .await
        .unwrap();

    assert!(
        started.elapsed() < Duration::from_millis(3000),
        "boot waited {:?} for a hanging LiveKit",
        started.elapsed()
    );
    assert!(app.voice_members(l.id).is_empty());
    assert_eq!(
        session(&app, a_id).await[0].2.as_deref(),
        Some("2026-01-01T00:00:00.000Z"),
        "fallback closed the old row at zero length"
    );
}

/// Host reboot where LiveKit starts after Pulse: the background retry finds the call later and
/// tells connected clients.
#[tokio::test]
async fn livekit_up_late_is_picked_up_by_the_retry_loop() {
    let app = spawn().await;
    let (a_id, a) = register(&app, "alex").await;
    let l = lounge(&app, &a).await;
    // Reserve a port, release it: nothing listens there until we "start LiveKit" below.
    let addr = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap();
    let s = state_with(&app, format!("http://{addr}"));
    let mut rx = app.hub.register(a_id, String::new()).rx;

    tokio::spawn(restore::retry_loop(
        s,
        pulse_server::db::now(),
        Duration::from_millis(100),
        50,
    ));
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(app.voice_members(l.id).is_empty(), "nothing to find yet");
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    serve_fake(listener, vec![(l.id.to_string(), vec![participant(a_id, "PA_1")])]);

    let frame = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("no VoiceJoined within 2 s of LiveKit coming up")
        .unwrap();
    match frame {
        ServerFrame::Event(Event::VoiceJoined { channel_id, user_id, .. }) => {
            assert_eq!((channel_id, user_id), (l.id, a_id));
        }
        other => panic!("unexpected {other:?}"),
    }
    assert!(app.voice_members(l.id).contains(&a_id));
    assert_eq!(session(&app, a_id).await.len(), 1, "a session row was opened");
}

#[tokio::test]
async fn server_boots_and_serves_when_livekit_is_down() {
    // spawn_binary points PULSE_LIVEKIT_API_URL at a closed port and waits for /health.
    let p = spawn_binary().await;
    let r = reqwest::get(p.url("/health")).await.unwrap();
    assert_eq!(r.status(), 200);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p pulse-server --test voice_restore`
Expected: compile errors: `could not find `restore` in `voice``, `no method named `state` found for
struct `TestApp``, `struct `Config` has no field named `livekit_api_url``, and `no method named
`restore``. `retry_loop` is also unresolved. After Step 3, `server_boots_and_serves_when_livekit_is_down`
guards the boot fallback. `hanging_livekit_at_boot_is_bounded_and_falls_back` proves the 2 s budget
covers a LiveKit that never answers, not only one that refuses the connection.
`livekit_up_late_is_picked_up_by_the_retry_loop` proves the background path. It can only be written
against a `retry_loop` whose interval and attempt count are parameters, which is why Step 3 pulls
it out of `on_startup`.

- [ ] **Step 3: Write minimal implementation**

`server/Cargo.toml` `[dependencies]`: change the `livekit-api` line and add `livekit-protocol`:

```toml
livekit-api = { version = "0.8.1", default-features = false, features = ["access-token", "webhooks", "services"] }
livekit-protocol = "0.8.0"
```

`server/src/config.rs`: add the field after `livekit_secret` (line 128):

```rust
    /// Where the server calls LiveKit's API (RoomService). Prod: loopback `http://`; the public
    /// `wss://` URL won't work here (no TLS stack for server-side API calls).
    pub livekit_api_url: String,
```

and in `from_env`, replace the `livekit_url` line (145) with:

```rust
            livekit_api_url: var(
                "PULSE_LIVEKIT_API_URL",
                &var("PULSE_LIVEKIT_URL", "ws://localhost:7880"),
            ),
            livekit_url: var("PULSE_LIVEKIT_URL", "ws://localhost:7880"),
```

`server/src/testing.rs`: in `test_config` (after `livekit_secret`, line 44):

```rust
        livekit_api_url: "http://127.0.0.1:1".into(), // LiveKit "down" unless a test overrides it
```

and in `impl TestApp` (after `voice_members`, line 35):

```rust
    /// The shared state the test server runs on, for calling startup tasks directly.
    pub fn state(&self) -> AppState {
        AppState {
            db: self.db.clone(),
            cfg: self.cfg.clone(),
            hub: self.hub.clone(),
            voice: self.voice.clone(),
            limits: self.limits.clone(),
        }
    }
```

`server/src/voice/state.rs`: replace the module doc (lines 1-3):

```rust
//! Who is in which voice room right now, with their mute/deafen flags. In memory; after a
//! restart it is rebuilt from LiveKit's room list (`voice::restore`), because LiveKit does NOT
//! re-report participants who were already connected.
```

and add after `join`:

```rust
    /// Add `user` from LiveKit's room list unless a webhook already placed them (its `sid` is
    /// newer than our snapshot). True if added.
    pub fn restore(&self, channel: ChannelId, user: UserId, sid: &str) -> bool {
        let flags = self.declared(user);
        let mut rooms = self.rooms.lock().unwrap();
        if rooms.values().any(|r| r.contains_key(&user)) {
            return false;
        }
        rooms.entry(channel).or_default().insert(
            user,
            Member {
                flags,
                sid: sid.to_owned(),
            },
        );
        true
    }
```

`server/src/voice/mod.rs`: line 2 becomes `pub mod restore;` followed by `pub mod state;`, and the doc
comment on `reconcile_on_startup` (lines 6-8) becomes:

```rust
/// Fallback when LiveKit can't be asked at startup: we cannot know when still-open sessions really
/// ended, so close them at zero length rather than let them run on and corrupt voice-hour stats.
```

Create `server/src/voice/restore.rs`:

```rust
//! Rebuild voice presence after a restart from LiveKit's own view of its rooms. A Pulse deploy
//! restarts the server but not LiveKit, so calls carry on and no `participant_joined` is re-sent.

use std::collections::HashSet;
use std::time::Duration;

use anyhow::Context;
use livekit_api::services::room::RoomClient;
use livekit_protocol::participant_info::State;
use pulse_protocol::gateway::Event;
use pulse_protocol::ids::{ChannelId, UserId};
use sqlx::SqlitePool;

use super::VoiceState;
use crate::AppState;
use crate::access::load_channel;

/// Startup waits this long for LiveKit before serving without it.
const BOOT_BUDGET: Duration = Duration::from_secs(2);
const RETRY_EVERY: Duration = Duration::from_secs(5);
const RETRIES: u32 = 12;

/// Someone LiveKit reports as connected to one of our rooms.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveParticipant {
    pub channel: ChannelId,
    pub user: UserId,
    pub sid: String,
    /// RFC 3339, from LiveKit's join time.
    pub joined_at: String,
}

/// Everyone connected, per LiveKit. Rooms are channel ids and identities user ids (our tokens,
/// `voice/routes.rs`); anything else isn't ours and is skipped. `budget` bounds the whole call.
pub async fn fetch(
    url: &str,
    key: &str,
    secret: &str,
    budget: Duration,
) -> anyhow::Result<Vec<LiveParticipant>> {
    let client = RoomClient::with_api_key(url, key, secret)
        .with_failover(false)
        .with_request_timeout(budget);
    tokio::time::timeout(budget, async {
        let mut out = Vec::new();
        for room in client.list_rooms(vec![]).await? {
            let Ok(channel) = room.name.parse::<ChannelId>() else {
                continue;
            };
            for p in client.list_participants(&room.name).await? {
                let Ok(user) = p.identity.parse::<UserId>() else {
                    continue;
                };
                if p.state == State::Disconnected as i32 {
                    continue;
                }
                let joined_at = chrono::DateTime::from_timestamp(p.joined_at, 0)
                    .map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
                    .unwrap_or_else(crate::db::now);
                out.push(LiveParticipant {
                    channel,
                    user,
                    sid: p.sid,
                    joined_at,
                });
            }
        }
        anyhow::Ok(out)
    })
    .await
    .context("LiveKit didn't answer in time")?
}

/// Make `voice` and `voice_sessions` agree with `live`. Session rows opened before `started_at`
/// (this boot) stay open for people still in a call and close at zero length for the rest (we
/// can't know when they left); server-channel calls with no open row get one from LiveKit's join
/// time. Returns who was newly added to `voice`.
pub async fn apply(
    db: &SqlitePool,
    voice: &VoiceState,
    live: &[LiveParticipant],
    started_at: &str,
) -> anyhow::Result<Vec<LiveParticipant>> {
    // Only rooms that are still channels; only server voice feeds stats (spec §8).
    let mut known = Vec::new();
    for p in live {
        if let Some(ch) = load_channel(db, p.channel).await? {
            known.push((p, ch.server_id.is_some()));
        }
    }
    let here: HashSet<(String, String)> = known
        .iter()
        .map(|(p, _)| (p.user.to_string(), p.channel.to_string()))
        .collect();

    let mut tx = crate::db::begin_write(db).await?;
    let open: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT id, user_id, channel_id FROM voice_sessions WHERE left_at IS NULL AND joined_at < ?",
    )
    .bind(started_at)
    .fetch_all(&mut *tx)
    .await?;
    for (id, user, channel) in open {
        if !here.contains(&(user, channel)) {
            sqlx::query("UPDATE voice_sessions SET left_at = joined_at WHERE id = ?")
                .bind(id)
                .execute(&mut *tx)
                .await?;
        }
    }
    for (p, record) in &known {
        if !record {
            continue;
        }
        let has_open: Option<i64> = sqlx::query_scalar(
            "SELECT 1 FROM voice_sessions WHERE user_id = ? AND channel_id = ? AND left_at IS NULL",
        )
        .bind(p.user.to_string())
        .bind(p.channel.to_string())
        .fetch_optional(&mut *tx)
        .await?;
        if has_open.is_none() {
            sqlx::query(
                "INSERT INTO voice_sessions (id, user_id, channel_id, joined_at) VALUES (?, ?, ?, ?)",
            )
            .bind(pulse_protocol::ids::next_ulid().to_string())
            .bind(p.user.to_string())
            .bind(p.channel.to_string())
            .bind(&p.joined_at)
            .execute(&mut *tx)
            .await?;
        }
    }
    tx.commit().await?;

    Ok(known
        .into_iter()
        .filter(|(p, _)| voice.restore(p.channel, p.user, &p.sid))
        .map(|(p, _)| p.clone())
        .collect())
}

/// Ask LiveKit, apply, and announce whoever was added (clients connected by then catch up).
pub async fn sync(s: &AppState, started_at: &str, budget: Duration) -> anyhow::Result<usize> {
    let live = fetch(
        &s.cfg.livekit_api_url,
        &s.cfg.livekit_key,
        &s.cfg.livekit_secret,
        budget,
    )
    .await?;
    let added = apply(&s.db, &s.voice, &live, started_at).await?;
    for p in &added {
        s.hub
            .publish(
                &s.db,
                Event::VoiceJoined {
                    channel_id: p.channel,
                    user_id: p.user,
                    flags: s.voice.declared(p.user),
                },
            )
            .await;
    }
    Ok(added.len())
}

/// Boot: one quick try before serving (so the first Ready after a deploy is right). If LiveKit
/// can't be reached, close old sessions as before, start empty, and keep trying in the background.
pub async fn on_startup(s: &AppState, started_at: &str) -> anyhow::Result<()> {
    match sync(s, started_at, BOOT_BUDGET).await {
        Ok(n) => {
            tracing::info!(restored = n, "voice presence restored from LiveKit");
            Ok(())
        }
        Err(e) => {
            tracing::warn!(error = %e, "LiveKit unreachable at startup; voice presence starts empty, retrying in the background");
            super::reconcile_on_startup(&s.db).await?;
            tokio::spawn(retry_loop(
                s.clone(),
                started_at.to_owned(),
                RETRY_EVERY,
                RETRIES,
            ));
            Ok(())
        }
    }
}

/// Background retries after a failed boot sync: every `every` (also each attempt's budget), up to
/// `tries` times. Stops at the first success, which announces whoever it found.
pub async fn retry_loop(s: AppState, started_at: String, every: Duration, tries: u32) {
    for attempt in 1..=tries {
        tokio::time::sleep(every).await;
        match sync(&s, &started_at, every).await {
            Ok(n) => {
                tracing::info!(restored = n, attempt, "voice presence restored from LiveKit");
                return;
            }
            Err(e) => tracing::debug!(error = %e, attempt, "LiveKit still unreachable"),
        }
    }
    tracing::warn!("gave up restoring voice presence; it fills in as people rejoin");
}
```

`server/src/lib.rs`, `serve` (lines 41-53) becomes:

```rust
pub async fn serve(cfg: config::Config) -> anyhow::Result<()> {
    let db = db::connect(&cfg.db_url).await?;
    let started_at = db::now();
    let listener = tokio::net::TcpListener::bind(cfg.bind).await?;
    tracing::info!(addr = %cfg.bind, "pulse-app-server listening");
    let hub = gateway::Hub::default();
    let state = AppState {
        db,
        limits: std::sync::Arc::new(limits::Limits::new(&cfg.limits)),
        cfg: std::sync::Arc::new(cfg),
        hub: hub.clone(),
        voice: voice::VoiceState::default(),
    };
    // Before serving (connections queue in the listen backlog), so the first Ready is right.
    voice::restore::on_startup(&state, &started_at).await?;
```

(the `axum::serve(...)` call below is unchanged; the old `voice::reconcile_on_startup(&db).await?;` at
line 43 is removed: it now runs only as the fallback inside `on_startup`.)

`deploy/compose.yml`, after line 19 (`PULSE_LIVEKIT_URL`):

```yaml
      # Server-to-LiveKit API (voice presence after a restart): loopback, plain HTTP.
      PULSE_LIVEKIT_API_URL: http://127.0.0.1:7880
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p pulse-server --test voice_restore`. All 8 tests should pass, the two new
failure-mode tests in under 3 s each. Then run `cargo test -p pulse-server` (including
`review_fixes::dangling_voice_sessions_closed_on_startup`, which still calls `reconcile_on_startup`),
`cargo clippy -p pulse-server --all-targets -- -D warnings`, `just deploy-check`, and check the
release image still builds (new feature pulls `reqwest` without TLS + `livekit-region`):
`cargo build --release -p pulse-server`.

Real-LiveKit check (local dev LiveKit only, never prod), after `just dev-livekit`:

```bash
TMP=$(mktemp -d)
export PULSE_DB_URL="sqlite://$TMP/r.db" PULSE_BIND=127.0.0.1:7890 PULSE_LIVEKIT_URL=ws://127.0.0.1:7880 \
       PULSE_LIVEKIT_KEY=devkey PULSE_LIVEKIT_SECRET=pulse-dev-secret-0123456789abcdefghij
cargo build -q -p pulse-server
./target/debug/pulse-app-server serve >"$TMP/1.log" 2>&1 & SRV=$!
sleep 1
# register + create a server as scripts/voice-smoke.sh does, giving $UID_ and $LOUNGE, then a peer that stays:
docker run -d --name pulse-lk-peer --network host -v "$PWD/spikes/voice:/w" livekit/livekit-cli:v2.18 room join \
  --url ws://127.0.0.1:7880 --api-key devkey --api-secret "$PULSE_LIVEKIT_SECRET" --identity "$UID_" \
  --publish /w/tone.ogg "$LOUNGE"
sleep 2; kill -INT $SRV; wait $SRV
./target/debug/pulse-app-server serve >"$TMP/2.log" 2>&1 & SRV=$!
sleep 2
grep "voice presence restored" "$TMP/2.log"          # expect restored=1
sqlite3 "$TMP/r.db" "select count(*) from voice_sessions where left_at is null"   # expect 1
docker rm -f pulse-lk-peer; kill -INT $SRV
```

- [ ] **Step 5: Commit**

```bash
git add server/Cargo.toml Cargo.lock server/src/config.rs server/src/testing.rs server/src/voice/mod.rs \
  server/src/voice/state.rs server/src/voice/restore.rs server/src/lib.rs deploy/compose.yml \
  server/tests/voice_restore.rs
git commit -m "feat(server): rebuild voice presence from LiveKit's room list at startup"
```

---

---

## Phase 2: Voice-pipeline foundation

### Task 5 (R3): Real-time priority for Windows audio threads (cpal 0.18.2 + MMCSS for our threads)

**Why:** opt-voice.md H3. Checked in the locked source: `~/.cargo/registry/src/*/cpal-0.15.3/src/host/wasapi/stream.rs:325-333`
calls `SetThreadPriority(HANDLE(GetCurrentThreadId() as isize), TIME_CRITICAL)`. A thread ID is not a
handle, so the call fails silently and the WASAPI callback threads stay at normal priority. cpal 0.18.2
(the current release, 2026-08-16) does `audio_thread_priority::promote_current_thread_to_real_time`
(MMCSS) behind the `realtime` feature, with a `SetThreadPriority(GetCurrentThread(), TIME_CRITICAL)`
fallback (`cpal-0.18.2/src/host/wasapi/stream.rs:714-728`). Our own `pulse-mic` (processor,
`processor.rs:253-263`) and `pulse-strong` (DeepFilterNet worker, `processor.rs:207-235`) run at normal
priority. When friends game while talking, this is the likeliest cause of glitches and MAX_LAG drops.
The `windows` crate 0.62.2 is already in `Cargo.lock` (tauri/wry, and it's what cpal 0.18.2 pins), so
MMCSS costs no new crate. cpal 0.15's `windows 0.54` drops out of the tree (one duplicate fewer).
Not in this task: the output callback's lock behaviour (the playout section owns that).

**API breaks to adapt (cpal 0.15 → 0.18, `UPGRADING.md` in the crate):** `SampleRate` is a plain `u32`;
`Device::name()` is gone (use `description()` / `id()`); per-operation errors become `cpal::Error`;
`build_*_stream` takes `StreamConfig` by value; `StreamInstant::duration_since` takes a value and saturates
(`checked_duration_since` keeps the `Option`); `SupportedStreamConfig` is `Copy` (clippy `clone_on_copy`
under `-D warnings`); `I24`/`U24` sample formats exist and now rank above `I16`. Two behaviour changes
matter for us. First, names. On Windows, 0.15's `name()` was `DEVPKEY_Device_FriendlyName`, and 0.18's
`description().name()` prefers the same property, so saved settings still match. On Linux (ALSA),
0.15's name was the PCM id (`"default"`, `"pipewire"`), but 0.18's `description().name()` is the first
line of the description, while `id().id()` is still the PCM id. `usable()` (`devices.rs:234-236`) and
saved settings depend on the PCM id, so Linux must use `id()`. Second, WASAPI timestamps now include
hardware latency, so `device ms in/out` on the path line reads higher than in the baseline without
anything having got worse.

**Files:**
- Modify: `client/src-tauri/Cargo.toml` (`cpal = "0.15"` line in `[dependencies]`; `[target.'cfg(windows)'.dependencies]` at the end)
- Modify: `client/src-tauri/src/voice/devices.rs:236-268` (`list`, `list_inputs`, `list_outputs`), `:578-620` (`open_half`), `:795-812` (`device_names`, `default_device_names`), `:825-860` (`prefer_internal_rate`, `find_device`), `:883-1000` (`stream_errors`, latencies, `build_input`, `build_output`), `:1549-1569` (test)
- Modify: `client/src-tauri/src/sounds.rs:122-127,147-152`
- Create: `client/src-tauri/src/voice/rtprio.rs`
- Modify: `client/src-tauri/src/voice/mod.rs:1-17` (add `pub mod rtprio;`)
- Modify: `client/src-tauri/src/voice/processor.rs:207-235` (`start_strong`), `:253-263` (`spawn`)
- Test: `client/src-tauri/src/voice/devices.rs` (tests module), `client/src-tauri/src/voice/rtprio.rs` (tests module)

**Interfaces:**
- Consumes: nothing new.
- Produces:
  - `pub fn voice::devices::device_name(d: &cpal::Device) -> Option<String>`: the one way to name a device (settings, lists, logs). Other sections must use it instead of `name()`.
  - `pub fn voice::rtprio::promote(what: &'static str) -> voice::rtprio::Registration`; `pub struct Registration { pub task: Option<&'static str> }`, which un-registers on drop (same thread; `!Send` on Windows).
  - The cpal 0.18 API for anyone touching `devices.rs`: `cpal::Error`, `u32` rates, configs passed by value.

- [ ] **Step 1: Write the failing tests**

In `client/src-tauri/src/voice/devices.rs`, replace the body of `prefers_internal_rate_when_supported` (`:1549-1569`) with the 0.18 API, and add a device-naming test after it:

```rust
    /// Opening at 48 kHz when the device can avoids resampling entirely (best quality, no work).
    #[test]
    fn prefers_internal_rate_when_supported() {
        use cpal::{SampleFormat::F32, SupportedBufferSize::Unknown};
        let default = cpal::SupportedStreamConfig::new(2, 44_100, Unknown, F32);
        let range =
            |ch, lo, hi| cpal::SupportedStreamConfigRange::new(ch, lo, hi, Unknown, F32);

        let picked = prefer_internal_rate(default, vec![range(2, 8_000, 192_000)]);
        assert_eq!(picked.sample_rate(), INTERNAL_RATE);
        assert_eq!(picked.channels(), 2, "keeps the default's channel count");

        let only_44k = prefer_internal_rate(default, vec![range(2, 44_100, 44_100)]);
        assert_eq!(only_44k, default, "can't do 48 kHz: keep the default");

        let other_channels = prefer_internal_rate(default, vec![range(6, 8_000, 192_000)]);
        assert_eq!(
            other_channels, default,
            "never trade channel layout for the rate"
        );
    }

    /// cpal 0.18 dropped `Device::name()`. Settings store names, so a device must keep the name
    /// 0.15 gave it: on Linux the ALSA PCM id (what `usable` filters on), not the description.
    /// Real devices: run explicitly (`--ignored`).
    #[cfg(target_os = "linux")]
    #[test]
    #[ignore]
    fn linux_device_names_stay_alsa_pcm_ids() {
        use cpal::traits::HostTrait;
        let host = cpal::default_host();
        let names: Vec<String> = host
            .output_devices()
            .into_iter()
            .flatten()
            .filter_map(|d| device_name(&d))
            .collect();
        assert!(!names.is_empty(), "no ALSA devices listed");
        for n in &names {
            assert!(!n.contains(' '), "a description, not a PCM id: {n:?}");
        }
        assert!(names.iter().any(|n| n == "default"), "{names:?}");
    }
```

Create `client/src-tauri/src/voice/rtprio.rs` with only the tests (the code follows in Step 3):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// cpal 0.15 passed a thread id where a handle belongs, so its WASAPI threads were never
    /// raised; 0.18 with `realtime` registers them with MMCSS. Guard against a downgrade.
    #[test]
    fn cpal_raises_its_own_wasapi_threads() {
        let manifest = include_str!("../../Cargo.toml");
        let line = manifest
            .lines()
            .find(|l| l.starts_with("cpal") && l.contains("realtime"))
            .expect("cpal with the `realtime` feature (Windows target dependency)");
        assert!(line.contains("\"0.18"), "{line}");
    }

    #[test]
    fn each_thread_kind_is_logged_once() {
        assert!(first_time("test-kind-a"));
        assert!(!first_time("test-kind-a"));
        assert!(first_time("test-kind-b"));
    }

    #[cfg(not(windows))]
    #[test]
    fn elsewhere_promotion_is_a_harmless_no_op() {
        let r = promote("test-noop");
        assert_eq!(r.task, None);
    }

    /// Runs only on Windows (no Windows CI: see Step 5 for the check on the owner's machine).
    #[cfg(windows)]
    #[test]
    fn windows_threads_join_mmcss() {
        let r = promote("test-mmcss");
        assert!(r.task.is_some(), "MMCSS refused this thread");
    }
}
```

and register it in `client/src-tauri/src/voice/mod.rs` (alphabetical, after `pub mod resampler;`):

```rust
pub mod rtprio;
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p pulse-client --lib voice::rtprio voice::devices::tests::prefers_internal_rate_when_supported`
Expected: compile errors. In `devices.rs`: `mismatched types: expected SampleRate, found integer` (cpal 0.15)
and `cannot find function device_name`. In `rtprio.rs`: `cannot find function first_time` / `promote`.

- [ ] **Step 3: Write minimal implementation**

`client/src-tauri/Cargo.toml`: replace `cpal = "0.15"` with `cpal = "0.18.2"` and add the Windows-only
feature (Cargo unifies features per target, so Linux builds stay feature-free; on Linux `realtime` is a
documented no-op anyway):

```toml
[target.'cfg(windows)'.dependencies]
tauri-plugin-global-shortcut = "2"
# 0.18's `realtime` raises cpal's own WASAPI threads via MMCSS (0.15 never did: thread id passed as a handle).
cpal = { version = "0.18.2", features = ["realtime"] }
# MMCSS for our own audio threads (voice/rtprio.rs); already in the tree via tauri and cpal.
windows = { version = "0.62", features = ["Win32_Foundation", "Win32_System_Threading"] }
```

Then `cargo update -p cpal` (Cargo.lock: cpal 0.18.2 in, cpal 0.15.3 and windows 0.54 out).

`client/src-tauri/src/voice/rtprio.rs` (above the tests):

```rust
//! Real-time scheduling for our own audio threads. On Windows they join the multimedia class
//! scheduler (MMCSS) as "Pro Audio" (or "Audio" if that's refused), so a game pinning the CPU
//! can't starve the mic processor or Strong's worker. cpal ≥ 0.18 with `realtime` does the same
//! for its own WASAPI threads. Elsewhere this is a no-op.

use std::sync::Mutex;

/// Keeps the calling thread registered; dropping it (on that same thread) unregisters.
#[must_use = "the thread leaves MMCSS when this is dropped"]
pub struct Registration {
    #[cfg(windows)]
    handle: Option<windows::Win32::Foundation::HANDLE>,
    /// The MMCSS task we got ("Pro Audio" / "Audio"), or None.
    pub task: Option<&'static str>,
}

/// Register the calling thread; `what` names it in the log (once per kind per run: the processor
/// thread is respawned for every call).
pub fn promote(what: &'static str) -> Registration {
    let (r, err) = register();
    if first_time(what) {
        match (r.task, err) {
            (Some(task), _) => {
                tracing::info!(thread = what, task, "audio thread: real-time priority (MMCSS)")
            }
            (None, Some(error)) => tracing::warn!(
                thread = what,
                %error,
                "audio thread: MMCSS refused; normal priority"
            ),
            (None, None) => tracing::debug!(thread = what, "audio thread: no MMCSS on this OS"),
        }
    }
    r
}

static LOGGED: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());

fn first_time(what: &'static str) -> bool {
    let mut seen = LOGGED.lock().unwrap_or_else(|e| e.into_inner());
    if seen.contains(&what) {
        false
    } else {
        seen.push(what);
        true
    }
}

#[cfg(windows)]
fn register() -> (Registration, Option<String>) {
    use windows::Win32::System::Threading::AvSetMmThreadCharacteristicsW;
    use windows::core::w;
    let mut last_err = None;
    for (name, task) in [("Pro Audio", w!("Pro Audio")), ("Audio", w!("Audio"))] {
        let mut index = 0u32;
        // SAFETY: `task` is a static NUL-terminated wide string and `index` a live out-pointer;
        // the returned handle is released by `Drop` on this same thread.
        match unsafe { AvSetMmThreadCharacteristicsW(task, &mut index) } {
            Ok(h) => {
                return (
                    Registration {
                        handle: Some(h),
                        task: Some(name),
                    },
                    None,
                );
            }
            Err(e) => last_err = Some(e.to_string()),
        }
    }
    (
        Registration {
            handle: None,
            task: None,
        },
        last_err,
    )
}

#[cfg(not(windows))]
fn register() -> (Registration, Option<String>) {
    (Registration { task: None }, None)
}

#[cfg(windows)]
impl Drop for Registration {
    fn drop(&mut self) {
        if let Some(h) = self.handle.take() {
            // SAFETY: `h` came from AvSetMmThreadCharacteristicsW on this thread (HANDLE is !Send,
            // so the Registration can't have moved threads).
            let _ = unsafe { windows::Win32::System::Threading::AvRevertMmThreadCharacteristics(h) };
        }
    }
}
```

`client/src-tauri/src/voice/processor.rs`. In `spawn` (`:258-261`):

```rust
    let h = std::thread::Builder::new()
        .name("pulse-mic".into())
        .spawn(move || {
            let _rt = super::rtprio::promote("pulse-mic");
            run(shared, rx, mic_tx)
        })
        .expect("spawn mic processor");
```

In `start_strong`, in the `Ok(mut df) =>` arm (`:214`), register **after** the model load (loading is
heavy and must not run at real-time priority), as the arm's first line:

```rust
            Ok(mut df) => {
                let _rt = super::rtprio::promote("pulse-strong");
                let (in_tx, in_rx) = std_mpsc::channel::<Vec<f32>>();
```

`client/src-tauri/src/voice/devices.rs`. Add the naming helper above `fn usable` (`:232`):

```rust
/// A device's name as settings and lists use it. cpal 0.18 dropped `name()`: on Linux (ALSA) the
/// stable name is the PCM id ("default", what 0.15 returned and `usable` filters on); elsewhere the
/// friendly name (`description().name()`, the same property 0.15 read on Windows).
pub fn device_name(d: &cpal::Device) -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        d.id().ok().map(|id| id.id().to_string())
    }
    #[cfg(not(target_os = "linux"))]
    {
        d.description().ok().map(|desc| desc.name().to_string())
    }
}
```

Mechanical replacements (each is the only change on its line):
- `:244` `it.filter_map(|d| d.name().ok())` → `it.filter_map(|d| device_name(&d))`
- `:259`, `:267`, `:809`, `:810` `.and_then(|d| d.name().ok())` → `.and_then(|d| device_name(&d))`
- `:599` `let (rate, ch) = (cfg.sample_rate().0, cfg.channels() as usize);` → `let (rate, ch) = (cfg.sample_rate(), cfg.channels() as usize);`
- `:615` `let name = dev.name().unwrap_or_default();` → `let name = device_name(&dev).unwrap_or_default();`
- `:797-798`:
  ```rust
      fn names(devs: Result<impl Iterator<Item = cpal::Device>, cpal::Error>) -> Vec<String> {
          devs.map(|d| d.filter_map(|d| device_name(&d)).collect())
  ```
- `:829` `let want = cpal::SampleRate(INTERNAL_RATE);` → `let want = INTERNAL_RATE;`
- `:847` `devs.find(|d| d.name().ok().as_deref() == Some(name))` → `devs.find(|d| device_name(d).as_deref() == Some(name))`
- `:885` `fn stream_errors(what: &'static str) -> impl FnMut(cpal::StreamError) + Send + 'static {` → `... impl FnMut(cpal::Error) + Send + 'static {`. (A refused real-time promotion now arrives here as `ErrorKind::RealtimeDenied` and is logged once by the existing throttle. That log line is the "cpal's thread was not raised" signal.)
- `:896-905`:
  ```rust
  fn capture_latency(info: &cpal::InputCallbackInfo) -> Option<Duration> {
      let t = info.timestamp();
      t.callback.checked_duration_since(t.capture)
  }

  /// How long until what we write is heard (device + driver buffering), if the backend says.
  fn playback_latency(info: &cpal::OutputCallbackInfo) -> Option<Duration> {
      let t = info.timestamp();
      t.playback.checked_duration_since(t.callback)
  }
  ```
- In both `typed` helpers (`:921`, `:966`): `dev.build_input_stream(cfg, ...)` / `dev.build_output_stream(cfg, ...)` → `(*cfg, ...)` (`StreamConfig` is `Copy`).
- In both F32 fast paths (`:935`, `:982`): `&c,` → `c,`.
- Add the 24-bit arms (0.18 ranks them above I16 for default configs). In `build_input` after `F::I16 => ...`:
  ```rust
          F::I24 => typed::<cpal::I24>(dev, &c, on_data),
          F::U24 => typed::<cpal::U24>(dev, &c, on_data),
  ```
  and in `build_output` after `F::I16 => ...`:
  ```rust
          F::I24 => typed::<cpal::I24>(dev, &c, fill),
          F::U24 => typed::<cpal::U24>(dev, &c, fill),
  ```

`client/src-tauri/src/sounds.rs`:
- `:126` `.filter(|d| n.is_none() || d.name().ok().as_deref() == n)` → `.filter(|d| n.is_none() || crate::voice::devices::device_name(d).as_deref() == n)`
- `:152` `let (rate, ch) = (cfg.sample_rate().0, cfg.channels() as usize);` → `let (rate, ch) = (cfg.sample_rate(), cfg.channels() as usize);`

Then `cargo clippy -p pulse-client --all-targets -- -D warnings` and fix what it names. Expect only
`clone_on_copy` on `SupportedStreamConfig`/`StreamConfig` clones and unused-import nits. If it names
anything else in `devices.rs`, adapt per `UPGRADING.md` sections 1-3 and do not change behaviour.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p pulse-client --lib voice::rtprio voice::devices`
Expected: PASS (`elsewhere_promotion_is_a_harmless_no_op`, `each_thread_kind_is_logged_once`, `cpal_raises_its_own_wasapi_threads`, `prefers_internal_rate_when_supported`, and the existing device tests).
Run (Linux dev box, real devices): `cargo test -p pulse-client --lib voice::devices::tests::linux_device_names_stay_alsa_pcm_ids -- --ignored`
Expected: PASS (names like `default`, `pipewire`, `hw:CARD=…`).
Run: `just dev-livekit && cargo test -p pulse-client --test voice_it real_devices_are_not_reopened_without_a_reason -- --ignored --nocapture`
Expected: PASS (0 reopens), and the `audio input opened device=default rate=48000` lines appear as before.
Run: `cargo tree -p pulse-client -i cpal` → `cpal v0.18.2` only. Run `cargo tree -d | grep -c "windows v0.54"` → `0`.
Run: `just check`. Expected: PASS.

- [ ] **Step 4b: Cover the new 24-bit path (I24/U24 round trip)**

cpal 0.18 can now hand us `I24`/`U24` default configs, which go through the new `typed::<cpal::I24>`
arms. A wrong conversion there means silence or full-scale noise from that headset. Add to the
`devices.rs` tests:

```rust
    fn round_trip<T: cpal::SizedSample + cpal::FromSample<f32>>(src: &[f32]) -> Vec<f32>
    where
        f32: cpal::FromSample<T>,
    {
        let mut dev = vec![<T as cpal::Sample>::EQUILIBRIUM; src.len()];
        from_f32(src, &mut dev);
        let mut back = Vec::new();
        to_f32(&dev, &mut back);
        back
    }

    /// cpal 0.18 ranks 24-bit formats above I16: a headset whose shared-mode format is 24-bit
    /// integer now goes through these conversions, which must be exact to 24-bit precision.
    #[test]
    fn twenty_four_bit_samples_round_trip() {
        let src = [-1.0f32, -0.5, 0.0, 0.5, 0.999];
        for (name, back) in [
            ("i24", round_trip::<cpal::I24>(&src)),
            ("u24", round_trip::<cpal::U24>(&src)),
        ] {
            assert_eq!(back.len(), src.len(), "{name}");
            for (a, b) in src.iter().zip(&back) {
                assert!((a - b).abs() < 1e-5, "{name}: {a} came back as {b}");
            }
        }
    }
```

Run: `cargo test -p pulse-client --lib voice::devices::tests::twenty_four_bit_samples_round_trip`
Expected: PASS. This is a coverage step: dasp's conversions do the work, and the test pins them. If it
fails, convert by hand in the two `F::I24`/`F::U24` arms instead. Read the samples as `i32`
(`I24::inner()`, `U24::inner() - (1 << 23)`) and scale by `1.0 / 8_388_608.0`, then re-run.

- [ ] **Step 5: Windows check (can't be unit-tested on Linux)**

`cargo check --target x86_64-pc-windows-msvc` doesn't work here: no `cargo-xwin`, no MSVC, and
webrtc-sys/ring build scripts need `cl.exe`. The repo's Windows build is the `release-windows`
workflow (`.github/workflows/release-windows.yml`, `workflow_dispatch`), which compiles the
`cfg(windows)` code. In the Windows build from the dev run, the check in the owner's test-call bug report is:
- the log has `audio thread: real-time priority (MMCSS) thread=pulse-mic task="Pro Audio"` once, and the same for `thread=pulse-strong` when Strong is on;
- the log has **no** `input stream error` / `output stream error` line mentioning real-time (cpal's own threads were raised);
- `drops lag` on the path lines while a game runs is compared against the 2026-10-15 baseline.
If MMCSS refuses (`audio thread: MMCSS refused`), the call still works at normal priority. Report the error text.

- [ ] **Step 6: Commit**

```bash
git add client/src-tauri/Cargo.toml Cargo.lock client/src-tauri/src/voice/devices.rs client/src-tauri/src/sounds.rs client/src-tauri/src/voice/rtprio.rs client/src-tauri/src/voice/mod.rs client/src-tauri/src/voice/processor.rs
git commit -m "perf(voice): real-time audio threads on Windows (cpal 0.18.2 realtime + MMCSS for mic and Strong)" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 6 (R4): NS attenuation and gate-open share on the `voice: path` line

**Why:** in the 2026-10-15 Windows baseline, a user saw Strong suppression clip word edges while a show
played through headphones. We have no numbers for what the denoiser removes or how often the gate
opens. The path line (`voice/mod.rs:459-466`, `stats.rs:121-143`) has CPU per stage
(`ns+gate NNus`) but nothing about what it did. Measure on the processor thread (`processor.rs:97-110`
for the chain, `:380-384` for the call site) with atomics only, the same pattern as `stats.rs`.

**Files:**
- Modify: `client/src-tauri/src/voice/stats.rs:55-118` (`Pipeline`, `take`, `Summary`), `:121-143` (`Display`), tests `:146-176`
- Modify: `client/src-tauri/src/voice/processor.rs:54-60` (`BlockOut`), `:97-110` (`MicChain::process`), `:380-384` (`run`)
- Test: `client/src-tauri/src/voice/stats.rs` (tests), `client/src-tauri/src/voice/processor.rs` (tests)

**Interfaces:**
- Consumes: `MicChain::process` (the gate section, H1, also edits this function: see Notes).
- Produces:
  - `pub fn stats::Pipeline::record_mic_block(&self, ns_in: f32, ns_out: f32, judged: bool, passed: bool)`
  - `stats::Summary { …, pub ns_atten_db: Option<f64>, pub gate_open_pct: Option<f64> }`. `ns_atten_db` is `10·log10(out/in)` over the window: negative means removed, floored at −60, `None` for a silent or all-muted window. `gate_open_pct` is the share of mic-open 10 ms blocks the gate passed.
  - `processor::BlockOut { level, sent, pub ns_in: f32, pub ns_out: f32, pub judged: bool }` (energies are sums of squares over the block).
  - Path line suffix: `… ns+gate 500us ns -12.3dB gate 45%` (`-` when not measured).

- [ ] **Step 1: Write the failing tests**

In `client/src-tauri/src/voice/stats.rs` tests, update the expected string in
`a_window_summarises_then_starts_over` to end with the new (unmeasured) fields:

```rust
        assert_eq!(
            s.to_string(),
            "frames in 480-1056 out - device ms in - out - recv 10/30/55ms underrun 15ms drops lag 2 send 0 fill 0 (x10ms) cpu apm - ns+gate 500us ns - gate -"
        );
```

and add:

```rust
    /// R4: what the denoiser removed and how often the gate let the mic through, per window.
    /// Muted blocks count for neither (the gate doesn't judge them, nothing is sent).
    #[test]
    fn denoiser_attenuation_and_gate_open_share_per_window() {
        let p = Pipeline::default();
        for k in 0..4 {
            p.record_mic_block(1.0, 0.1, true, k == 0); // −10 dB, gate passed 1 of 4
        }
        p.record_mic_block(1.0, 1.0, false, false); // muted: ignored
        let s = p.take(0);
        assert_eq!(s.ns_atten_db.map(|v| (v * 10.0).round() / 10.0), Some(-10.0));
        assert_eq!(s.gate_open_pct.map(f64::round), Some(25.0));
        assert!(s.to_string().ends_with(" ns -10.0dB gate 25%"), "{s}");
        let next = p.take(0);
        assert_eq!((next.ns_atten_db, next.gate_open_pct), (None, None));
    }

    #[test]
    fn fully_removed_reads_as_the_floor_and_silence_as_unmeasured() {
        let p = Pipeline::default();
        p.record_mic_block(1.0, 0.0, true, false);
        let s = p.take(0);
        assert_eq!(s.ns_atten_db, Some(-60.0));
        assert_eq!(s.gate_open_pct, Some(0.0));
        p.record_mic_block(0.0, 0.0, true, false); // digital silence in
        assert_eq!(p.take(0).ns_atten_db, None);
    }
```

In `client/src-tauri/src/voice/processor.rs` tests add:

```rust
    struct Halve;
    impl Denoiser for Halve {
        fn process(&mut self, block: &mut [f32]) -> Option<f32> {
            block.iter_mut().for_each(|v| *v *= 0.5);
            Some(1.0)
        }
    }

    /// R4: each block reports the denoiser's input and output energy, and whether the gate judged it.
    #[test]
    fn block_out_reports_denoiser_energy_and_whether_the_gate_judged() {
        let mut c = MicChain::new(NsLevel::Off, false, 0.0, 1.0);
        c.set_denoiser(NsLevel::Standard, Box::new(Halve));
        let mut b = vec![0.2f32; FRAME];
        let out = c.process(&mut b, true, Instant::now());
        // 480 × 0.04 = 19.2 in; 480 × 0.01 = 4.8 out (−6 dB)
        assert!((out.ns_in - 19.2).abs() < 1e-3, "{}", out.ns_in);
        assert!((out.ns_out - 4.8).abs() < 1e-3, "{}", out.ns_out);
        assert!(out.judged);
        let out = c.process(&mut vec![0.2f32; FRAME], false, Instant::now());
        assert!(!out.judged, "muted: the gate didn't judge it");
    }

    /// R4 end to end: the processor feeds the path-line counters.
    #[test]
    fn processor_counts_denoiser_energy_and_gate_verdicts() {
        let shared = shared_with(AudioConfig {
            noise_suppression: NsLevel::Off,
            auto_sensitivity: false,
            sensitivity: 0.0, // manual, threshold 0 = always open
            ..Default::default()
        });
        let (mic_tx, mut mic_rx) = tokio::sync::mpsc::unbounded_channel();
        let (raw_tx, _h) = spawn(shared.clone(), mic_tx);
        let tone: Vec<f32> = (0..480).map(|i| 0.1 * (i as f32 * 0.0576).sin()).collect();
        for _ in 0..10 {
            raw_tx.send((48_000, tone.clone(), Instant::now())).unwrap();
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(recv_n(&mut mic_rx, 10).len(), 10);
        let s = shared.stats.take(0);
        assert_eq!(s.ns_atten_db.map(f64::round), Some(0.0), "Off removes nothing");
        assert_eq!(s.gate_open_pct.map(f64::round), Some(100.0));
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p pulse-client --lib voice::stats voice::processor`
Expected: compile errors: `no method named record_mic_block`, `no field ns_atten_db on type Summary`, `no field ns_in on type BlockOut`.

- [ ] **Step 3: Write minimal implementation**

`client/src-tauri/src/voice/stats.rs`. Add to `Pipeline` (after `pub chain: StageTime,`):

```rust
    /// The denoiser's input and output energy (sum of squares × ENERGY_SCALE), mic open only.
    ns_in: AtomicU64,
    ns_out: AtomicU64,
    /// Mic-open 10 ms blocks the gate judged, and how many it let through.
    gate_judged: AtomicU64,
    gate_passed: AtomicU64,
```

Add above `impl Pipeline`:

```rust
/// Block energies are f32 sums of squares; stored as integers so the audio side stays lock-free.
const ENERGY_SCALE: f64 = 1e9;
/// Below this the window had no real input (≈ −100 dBFS over a 5 s window).
const SILENT_ENERGY: u64 = 1_000;
/// "Removed everything" reads as this instead of −∞.
const ATTEN_FLOOR_DB: f64 = -60.0;

/// Level change through the denoiser over the window, in dB (negative = removed).
fn atten_db(e_in: u64, e_out: u64) -> Option<f64> {
    (e_in >= SILENT_ENERGY)
        .then(|| (10.0 * (e_out.max(1) as f64 / e_in as f64).log10()).max(ATTEN_FLOOR_DB))
}
```

Add to `impl Pipeline`:

```rust
    /// One processed mic block: the denoiser's input/output energy and the gate's verdict. Muted
    /// blocks (`judged == false`) don't count: nothing was judged or sent.
    pub fn record_mic_block(&self, ns_in: f32, ns_out: f32, judged: bool, passed: bool) {
        if !judged {
            return;
        }
        self.ns_in
            .fetch_add((ns_in as f64 * ENERGY_SCALE) as u64, Relaxed);
        self.ns_out
            .fetch_add((ns_out as f64 * ENERGY_SCALE) as u64, Relaxed);
        self.gate_judged.fetch_add(1, Relaxed);
        if passed {
            self.gate_passed.fetch_add(1, Relaxed);
        }
    }
```

In `take`, before `Summary {`:

```rust
        let (ns_in, ns_out) = (self.ns_in.swap(0, Relaxed), self.ns_out.swap(0, Relaxed));
        let (judged, passed) = (
            self.gate_judged.swap(0, Relaxed),
            self.gate_passed.swap(0, Relaxed),
        );
```

and in the `Summary { … }` literal after `chain_us: …`:

```rust
            ns_atten_db: atten_db(ns_in, ns_out),
            gate_open_pct: (judged > 0).then(|| passed as f64 * 100.0 / judged as f64),
```

Add the fields to `pub struct Summary` (after `pub chain_us: Option<u64>,`):

```rust
    /// Denoiser output vs input power over the window, dB (negative = removed; floor −60).
    pub ns_atten_db: Option<f64>,
    /// Share of mic-open 10 ms blocks the voice gate let through.
    pub gate_open_pct: Option<f64>,
```

In `impl fmt::Display for Summary`, extend the format string and arguments:

```rust
        let ns = self
            .ns_atten_db
            .map_or("-".into(), |v| format!("{v:.1}dB"));
        let gate = self
            .gate_open_pct
            .map_or("-".into(), |v| format!("{v:.0}%"));
        write!(
            f,
            "frames in {} out {} device ms in {} out {} recv {recv} underrun {}ms drops lag {} send {} fill {} (x10ms) cpu apm {} ns+gate {} ns {ns} gate {gate}",
            // (existing arguments unchanged)
```

`client/src-tauri/src/voice/processor.rs`. `BlockOut` (`:54-60`):

```rust
pub struct BlockOut {
    /// RMS after cleaning + gain (the mic meter: what you see is what is sent).
    pub level: f32,
    /// Whether audio (not silence) went out.
    pub sent: bool,
    /// Sum of squares into and out of the denoiser (its attenuation, for the path line).
    pub ns_in: f32,
    pub ns_out: f32,
    /// Whether the gate judged this block (false while muted).
    pub judged: bool,
}
```

`MicChain::process` (`:97-110`):

```rust
    pub fn process(&mut self, block: &mut [f32], mic_open: bool, now: Instant) -> BlockOut {
        let energy = |b: &[f32]| b.iter().map(|v| v * v).sum::<f32>();
        let ns_in = energy(block);
        let prob = self.denoiser.process(block);
        let ns_out = energy(block);
        for v in block.iter_mut() {
            *v = soft_limit(*v * self.gain);
        }
        let level = (block.iter().map(|v| v * v).sum::<f32>() / block.len() as f32).sqrt();
        if !mic_open {
            self.gate.reset();
            block.fill(0.0);
            return BlockOut { level, sent: false, ns_in, ns_out, judged: false };
        }
        let sent = self.gate.process(block, prob, now);
        BlockOut { level, sent, ns_in, ns_out, judged: true }
    }
```

In `run`, right after `shared.stats.chain.record(t.elapsed());` (`:383`):

```rust
            shared
                .stats
                .record_mic_block(out.ns_in, out.ns_out, out.judged, out.sent);
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p pulse-client --lib voice::stats voice::processor`
Expected: PASS (the new tests plus the existing processor and stats tests).
Run: `cargo clippy -p pulse-client --all-targets -- -D warnings`. Expected: clean.

- [ ] **Step 5: Commit**

```bash
git add client/src-tauri/src/voice/stats.rs client/src-tauri/src/voice/processor.rs
git commit -m "feat(diagnostics): noise-suppression attenuation and gate-open share on the voice: path line" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 7 (R5): Is NetEq's "buffer" a real standing delay? Fix the window deltas, measure against real lag

**Why:** in the baseline, `buffer` read 80-100 ms while `target` read 20-40 ms and `jitter` about 1 ms.
How the path line computes them (`voice/netstats.rs:42-97`): for every inbound audio `InboundRtp` it
**sums across peers** `jitterBufferDelay`, `jitterBufferTargetDelay`, `jitterBufferMinimumDelay` and
`jitterBufferEmittedCount` (`:63-71`). It then divides the **window delta** of each sum by the delta of
emitted samples (`:79-87`). So it is already per window, not a lifetime average, and our own queues are
not in it (the mixer queue is `recv` on the other half of the line). Three things are still open:
- (a) Summing across peers is wrong when one peer leaves and another talks in the same window. The guard at `:79` only catches totals that shrink. Example: peers 10 s/100 k and 1 s/10 k, then peer 1 leaves while peer 2 adds 240 k samples at 20 ms each. The summed delta reads 34 ms instead of 20.
- (b) DTX. Our gate sends exact zeros, so Opus DTX runs during silence. NetEq's comfort-noise and SID handling could weigh long waits into `jitterBufferDelay` without being a delay anyone hears.
- (c) A real standing excess: if it is real, NetEq would be accelerating (`removedSamplesForAcceleration`) all the time to drain it.

The investigation fixes (a) unconditionally. It adds per-window `accel`/`decel`/`silent` so (b) and (c)
can be told apart in any bug report. Then it measures the **actual** lag of the bare LiveKit path on
this machine and compares it with `buffer` and `target`.

**Files:**
- Modify: `client/src-tauri/src/voice/netstats.rs:11-21` (`Totals`), `:23-40` (`PathStats`), `:42-97` (`summarize`), `:109-127` (`Display`), tests `:130-247`
- Modify: `client/src-tauri/src/voice/mod.rs:459-460` (call site passes `&totals`), `:935` (`mic_publish_options` → `pub`)
- Create: `client/src-tauri/tests/neteq_probe.rs` (ignored probe, needs `just dev-livekit`)
- Create: `docs/superpowers/research/opt-neteq-buffer.md` (the finding)

**Interfaces:**
- Consumes: `livekit::webrtc::stats::InboundRtpStats` fields `jitter_buffer_*`, `silent_concealed_samples`, `removed_samples_for_acceleration`, `inserted_samples_for_deceleration` (libwebrtc 0.3.50 `src/stats.rs:378-387`).
- Produces:
  - `pub fn netstats::summarize(publisher: &[RtcStats], subscriber: &[RtcStats], prev: &Totals) -> (PathStats, Totals)`. `prev` is now **by reference**, and `Totals` is `Clone`, not `Copy`.
  - `PathStats { …, pub accel_ms: Option<f64>, pub decel_ms: Option<f64>, pub silent_pct: Option<f64> }`; the path-line suffix is `… send 50 pkt/s accel 0ms decel 0ms silent 12.0%`.
  - `pub fn voice::mic_publish_options() -> TrackPublishOptions` (was private; the probe publishes exactly like the app).
  - The note `docs/superpowers/research/opt-neteq-buffer.md` with a `Verdict:` line of `artifact` or `standing-delay`. The playout section reads it.

- [ ] **Step 1: Write the failing tests**

In `client/src-tauri/src/voice/netstats.rs` tests: change every `Totals::default()` argument to
`&Totals::default()` and every `first` argument to `&first` (call sites in
`reads_rtt_from_the_active_pair_only`, `jitter_is_the_worst_peer_and_loss_is_summed`,
`buffer_delay_is_the_average_since_the_last_sample`, `nothing_measured_yet_prints_dashes`,
`jitter_buffer_aim_concealment_and_send_rate_since_the_last_sample`). Update the dash test's
expected string to:
`"rtt - jitter - buffer - target - min - concealed - lost 0 send - pkt/s accel - decel - silent -"`. Then add:

```rust
    fn inbound(ssrc: u32, set: impl FnOnce(&mut InboundRtpStats)) -> RtcStats {
        let mut r = InboundRtpStats::default();
        r.stream.kind = "audio".into();
        r.stream.ssrc = ssrc;
        set(&mut r);
        RtcStats::InboundRtp(r)
    }

    fn jb(ssrc: u32, delay_s: f64, target_s: f64, emitted: u64) -> RtcStats {
        inbound(ssrc, |r| {
            r.inbound.jitter_buffer_delay = delay_s;
            r.inbound.jitter_buffer_target_delay = target_s;
            r.inbound.jitter_buffer_emitted_count = emitted;
        })
    }

    /// R5 (a): a window's buffer is per stream. One peer leaving while another talks must not skew
    /// it (the summed totals read 34 ms here instead of 20).
    #[test]
    fn buffer_is_a_per_stream_window_delta_across_peer_churn() {
        let (_, first) = summarize(
            &[],
            &[jb(1, 10.0, 5.0, 100_000), jb(2, 1.0, 0.5, 10_000)],
            &Totals::default(),
        );
        // peer 1 left; peer 2 emitted 240 000 more samples that waited 20 ms each (aim 10 ms)
        let (s, _) = summarize(&[], &[jb(2, 4801.0, 2400.5, 250_000)], &first);
        assert_eq!(s.buffer_ms.map(f64::round), Some(20.0));
        assert_eq!(s.target_ms.map(f64::round), Some(10.0));
    }

    /// A peer that rejoins arrives as a new stream (new SSRC) whose counters start over: all of
    /// them belong to this window (summed totals "shrank" and the window was skipped).
    #[test]
    fn a_new_stream_counts_from_zero() {
        let (_, first) = summarize(&[], &[jb(1, 100.0, 50.0, 48_000)], &Totals::default());
        let (s, _) = summarize(&[], &[jb(3, 4.8, 2.4, 240_000)], &first);
        assert_eq!(s.buffer_ms.map(f64::round), Some(20.0));
    }

    /// R5 (b)/(c): time-stretching and comfort noise per window. Is NetEq draining a standing
    /// excess (acceleration), or is the peer mostly silent (DTX)?
    #[test]
    fn acceleration_and_silent_share_per_window() {
        let at = |emitted, accel, silent, received| {
            inbound(7, |r| {
                r.inbound.jitter_buffer_emitted_count = emitted;
                r.inbound.removed_samples_for_acceleration = accel;
                r.inbound.silent_concealed_samples = silent;
                r.inbound.total_samples_received = received;
            })
        };
        let (_, first) = summarize(&[], &[at(48_000, 0, 0, 48_000)], &Totals::default());
        // 5 s later: 2 400 samples (50 ms at 48 kHz) cut by acceleration; 60 000 of 240 000 silent
        let (s, _) = summarize(&[], &[at(288_000, 2_400, 60_000, 288_000)], &first);
        assert_eq!(s.accel_ms.map(f64::round), Some(50.0));
        assert_eq!(s.decel_ms.map(f64::round), Some(0.0));
        assert_eq!(s.silent_pct.map(f64::round), Some(25.0));
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p pulse-client --lib voice::netstats`
Expected: compile errors (`expected Totals, found &Totals`; `no field accel_ms on type PathStats`). With
only the `&` changes applied, `buffer_is_a_per_stream_window_delta_across_peer_churn` fails with
`left: Some(34.0) right: Some(20.0)` and `a_new_stream_counts_from_zero` with `left: None`.

- [ ] **Step 3: Write minimal implementation**

`client/src-tauri/src/voice/netstats.rs`. Replace `Totals` (`:11-21`) with:

```rust
use std::collections::HashMap;

use livekit::webrtc::stats::InboundRtpStats;

/// One inbound audio stream's counters. WebRTC's jitter-buffer figures are running totals since
/// the stream started, so a window's numbers are the difference between two samples of the SAME
/// stream (by SSRC). Summing across peers breaks when one leaves and another talks.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
struct Inbound {
    buffer_delay_s: f64,
    target_delay_s: f64,
    min_delay_s: f64,
    emitted: u64,
    received: u64,
    concealed: u64,
    silent_concealed: u64,
    accelerated: u64,
    decelerated: u64,
}

impl Inbound {
    fn of(r: &InboundRtpStats) -> Self {
        let i = &r.inbound;
        Self {
            buffer_delay_s: i.jitter_buffer_delay,
            target_delay_s: i.jitter_buffer_target_delay,
            min_delay_s: i.jitter_buffer_minimum_delay,
            emitted: i.jitter_buffer_emitted_count,
            received: i.total_samples_received,
            concealed: i.concealed_samples,
            silent_concealed: i.silent_concealed_samples,
            accelerated: i.removed_samples_for_acceleration,
            decelerated: i.inserted_samples_for_deceleration,
        }
    }

    /// This window's share: `self` minus the same stream's previous sample. A stream not seen
    /// before, or one whose counters went backwards (restarted), counts in full.
    fn since(&self, before: Option<&Inbound>) -> Inbound {
        let Some(b) = before else { return *self };
        if self.emitted < b.emitted
            || self.received < b.received
            || self.buffer_delay_s < b.buffer_delay_s
        {
            return *self;
        }
        Inbound {
            buffer_delay_s: self.buffer_delay_s - b.buffer_delay_s,
            target_delay_s: (self.target_delay_s - b.target_delay_s).max(0.0),
            min_delay_s: (self.min_delay_s - b.min_delay_s).max(0.0),
            emitted: self.emitted - b.emitted,
            received: self.received - b.received,
            concealed: self.concealed.saturating_sub(b.concealed),
            silent_concealed: self.silent_concealed.saturating_sub(b.silent_concealed),
            accelerated: self.accelerated.saturating_sub(b.accelerated),
            decelerated: self.decelerated.saturating_sub(b.decelerated),
        }
    }

    fn add(&mut self, d: &Inbound) {
        self.buffer_delay_s += d.buffer_delay_s;
        self.target_delay_s += d.target_delay_s;
        self.min_delay_s += d.min_delay_s;
        self.emitted += d.emitted;
        self.received += d.received;
        self.concealed += d.concealed;
        self.silent_concealed += d.silent_concealed;
        self.accelerated += d.accelerated;
        self.decelerated += d.decelerated;
    }
}

/// Each stream's counters at the previous sample.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Totals {
    inbound: HashMap<u32, Inbound>,
    packets_sent: u64,
}

/// Opus runs NetEq at 48 kHz: samples per millisecond.
const SAMPLES_PER_MS: f64 = 48.0;
```

Add to `PathStats` (after `pub send_pps: Option<f64>,`):

```rust
    /// Audio NetEq cut (acceleration) or stretched (deceleration) this window, in ms: a standing
    /// excess over target shows up as steady acceleration.
    pub accel_ms: Option<f64>,
    pub decel_ms: Option<f64>,
    /// Share of received audio that was silent concealment (DTX / comfort noise).
    pub silent_pct: Option<f64>,
```

Replace `summarize` (`:42-97`):

```rust
pub fn summarize(
    publisher: &[RtcStats],
    subscriber: &[RtcStats],
    prev: &Totals,
) -> (PathStats, Totals) {
    let mut out = PathStats::default();
    let mut totals = Totals::default();
    let mut window = Inbound::default();
    for s in publisher.iter().chain(subscriber) {
        match s {
            RtcStats::CandidatePair(p) => {
                let p = &p.candidate_pair;
                if p.nominated
                    && p.state == Some(IceCandidatePairState::Succeeded)
                    && p.current_round_trip_time > 0.0
                {
                    let rtt = p.current_round_trip_time * 1000.0;
                    out.rtt_ms = Some(out.rtt_ms.map_or(rtt, |r: f64| r.max(rtt)));
                }
            }
            RtcStats::InboundRtp(r) if r.stream.kind == "audio" => {
                let jitter = r.received.jitter * 1000.0;
                out.jitter_ms = Some(out.jitter_ms.map_or(jitter, |j: f64| j.max(jitter)));
                out.lost += r.received.packets_lost;
                let now = Inbound::of(r);
                window.add(&now.since(prev.inbound.get(&r.stream.ssrc)));
                totals.inbound.insert(r.stream.ssrc, now);
            }
            RtcStats::OutboundRtp(o) if o.stream.kind == "audio" => {
                totals.packets_sent += o.sent.packets_sent;
            }
            _ => {}
        }
    }
    if window.emitted > 0 {
        let per_sample = |s: f64| s / window.emitted as f64 * 1000.0;
        out.buffer_ms = Some(per_sample(window.buffer_delay_s));
        out.target_ms = Some(per_sample(window.target_delay_s));
        out.min_ms = Some(per_sample(window.min_delay_s));
        out.accel_ms = Some(window.accelerated as f64 / SAMPLES_PER_MS);
        out.decel_ms = Some(window.decelerated as f64 / SAMPLES_PER_MS);
    }
    if window.received > 0 {
        let pct = |n: u64| n as f64 / window.received as f64 * 100.0;
        out.concealed_pct = Some(pct(window.concealed));
        out.silent_pct = Some(pct(window.silent_concealed));
    }
    if totals.packets_sent > prev.packets_sent && prev.packets_sent > 0 {
        let sent = (totals.packets_sent - prev.packets_sent) as f64;
        out.send_pps = Some(sent / WINDOW.as_secs_f64());
    }
    (out, totals)
}
```

In `impl fmt::Display for PathStats`, extend:

```rust
        write!(
            f,
            "rtt {} jitter {} buffer {} target {} min {} concealed {} lost {} send {} pkt/s accel {} decel {} silent {}",
            ms(self.rtt_ms),
            ms(self.jitter_ms),
            ms(self.buffer_ms),
            ms(self.target_ms),
            ms(self.min_ms),
            pct(self.concealed_pct),
            self.lost,
            pps,
            ms(self.accel_ms),
            ms(self.decel_ms),
            pct(self.silent_pct),
        )
```

`client/src-tauri/src/voice/mod.rs:459-460`:

```rust
                        let (line, t) =
                            netstats::summarize(&s.publisher_stats, &s.subscriber_stats, &totals);
```

and `:935` `fn mic_publish_options()` → `pub fn mic_publish_options()` (doc line added: `/// Also used by tests/neteq_probe.rs, so the probe sends exactly like the app.`).

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p pulse-client --lib voice::netstats`
Expected: PASS (8 tests).

- [ ] **Step 5: Write the reproduction probe**

Create `client/src-tauri/tests/neteq_probe.rs`. It measures the **actual** onset-to-arrival lag of the bare
LiveKit path between two participants in this process (same clock, no Pulse pipeline) and prints it
next to the path-line numbers for the same windows. There are two send patterns. `Continuous` has
onsets with no DTX, so it gives the true jitter-buffer level. `Bursts` has exact-zero gaps like our
gate, so DTX engages.

```rust
//! R5 probe: is NetEq's "buffer" (jitterBufferDelay / emitted) a real standing delay? Measures the
//! actual onset-to-arrival lag of the bare LiveKit path (no Pulse pipeline) between two
//! participants in this process and prints it next to the path-line numbers.
//! Needs the dev LiveKit (`just dev-livekit`, host networking on lo).
//! Run: `cargo test -p pulse-client --test neteq_probe -- --ignored --nocapture`
//! With delay: `sudo tc qdisc add dev lo root netem delay 40ms 1ms` first and
//! `NETEM_ONEWAY_MS=40` in the environment; `sudo tc qdisc del dev lo root` after.

use std::time::{Duration, Instant};

use futures::StreamExt;
use livekit::prelude::*;
use livekit::webrtc::audio_frame::AudioFrame;
use livekit::webrtc::audio_source::native::NativeAudioSource;
use livekit::webrtc::audio_source::{AudioSourceOptions, RtcAudioSource};
use livekit::webrtc::audio_stream::native::NativeAudioStream;
use pulse_client::api::Api;
use pulse_client::voice::{mic_publish_options, netstats};
use pulse_server::testing;
use tokio::sync::mpsc;

const DEV_SECRET: &str = "pulse-dev-secret-0123456789abcdefghij";
const RATE: u32 = 48_000;
const FRAME: usize = 480;
const SECONDS: u64 = 30;
/// Onset to arrival that isn't the jitter buffer: ~5 ms waiting for the 20 ms Opus packet to
/// fill, ~5 ms for the receiver's 10 ms pull. ±5 ms; the question is a 50+ ms gap.
const PATH_FIXED_MS: f64 = 10.0;
/// Received 10 ms frames louder than this count as "on".
const ON_RMS: f64 = 0.05;

#[derive(Clone, Copy, Debug)]
enum Pattern {
    /// 0.8 s loud tone, 0.2 s quiet tone (−20 dB, not zero: no DTX). One onset per second.
    Continuous,
    /// 1 s loud tone, 1 s exact zeros: DTX engages in every gap, like our gate.
    Bursts,
}

impl Pattern {
    /// Tone amplitude for 10 ms frame `k`.
    fn amp(self, k: u64) -> f32 {
        match self {
            Pattern::Continuous => {
                if k % 100 < 20 {
                    0.03
                } else {
                    0.3
                }
            }
            Pattern::Bursts => {
                if k % 200 < 100 {
                    0.3
                } else {
                    0.0
                }
            }
        }
    }

    fn onset(self, k: u64) -> bool {
        k > 0 && self.amp(k) > 0.1 && self.amp(k - 1) < 0.1
    }
}

async fn send(source: NativeAudioSource, pattern: Pattern, onsets: mpsc::UnboundedSender<Instant>) {
    let mut tick = tokio::time::interval(Duration::from_millis(10));
    let step = 2.0 * std::f32::consts::PI * 440.0 / RATE as f32;
    let mut phase = 0f32;
    for k in 0..SECONDS * 100 {
        tick.tick().await;
        let a = pattern.amp(k);
        let data: Vec<i16> = (0..FRAME)
            .map(|_| {
                phase = (phase + step) % (2.0 * std::f32::consts::PI);
                (phase.sin() * a * 32767.0) as i16
            })
            .collect();
        if pattern.onset(k) {
            let _ = onsets.send(Instant::now());
        }
        let frame = AudioFrame {
            data: data.into(),
            sample_rate: RATE,
            num_channels: 1,
            samples_per_channel: FRAME as u32,
        };
        let _ = source.capture_frame(&frame).await;
    }
}

fn percentile(sorted: &[f64], p: f64) -> Option<f64> {
    (!sorted.is_empty()).then(|| sorted[((sorted.len() - 1) as f64 * p).round() as usize])
}

fn median(mut v: Vec<f64>) -> Option<f64> {
    v.sort_by(f64::total_cmp);
    percentile(&v, 0.5)
}

async fn run(
    pattern: Pattern,
    source: &NativeAudioSource,
    receiver: &Room,
    arrivals: &mut mpsc::UnboundedReceiver<Instant>,
) {
    while arrivals.try_recv().is_ok() {} // leftovers from the previous pattern
    let (sent_tx, mut sent_rx) = mpsc::unbounded_channel();
    let sending = tokio::spawn(send(source.clone(), pattern, sent_tx));
    let mut totals = netstats::Totals::default();
    let mut windows = Vec::new();
    let mut tick = tokio::time::interval(netstats::WINDOW);
    tick.tick().await;
    for _ in 0..SECONDS / 5 {
        tick.tick().await;
        if let Ok(s) = receiver.get_stats().await {
            let (p, t) = netstats::summarize(&s.publisher_stats, &s.subscriber_stats, &totals);
            totals = t;
            windows.push(p);
        }
    }
    sending.await.unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await; // last arrivals in flight
    let sent: Vec<Instant> = std::iter::from_fn(|| sent_rx.try_recv().ok()).collect();
    let got: Vec<Instant> = std::iter::from_fn(|| arrivals.try_recv().ok()).collect();
    let mut lags: Vec<f64> = got
        .iter()
        .filter_map(|g| {
            sent.iter()
                .rev()
                .find(|s| **s <= *g)
                .map(|s| g.duration_since(*s).as_secs_f64() * 1000.0)
        })
        .filter(|ms| *ms < 900.0)
        .collect();
    lags.sort_by(f64::total_cmp);
    let netem: f64 = std::env::var("NETEM_ONEWAY_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0.0);
    // the first window includes the subscription start-up: leave it out
    let steady = || windows.iter().skip(1);
    let col = |f: fn(&netstats::PathStats) -> Option<f64>| {
        median(steady().filter_map(f).collect()).map_or("-".into(), |v| format!("{v:.0}"))
    };
    let lag50 = percentile(&lags, 0.5);
    println!(
        "| {pattern:?} | {netem:.0} | {} | {} | {} | {} | {} | {} | {} | {} |",
        lag50.map_or("-".into(), |v| format!("{v:.0}")),
        percentile(&lags, 0.9).map_or("-".into(), |v| format!("{v:.0}")),
        lag50.map_or("-".into(), |v| format!("{:.0}", v - netem - PATH_FIXED_MS)),
        col(|p| p.buffer_ms),
        col(|p| p.target_ms),
        col(|p| p.accel_ms),
        col(|p| p.silent_pct),
        lags.len(),
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn neteq_buffer_vs_measured_lag() {
    let mut cfg = testing::test_config();
    cfg.livekit_url = "ws://127.0.0.1:7880".into();
    cfg.livekit_secret = DEV_SECRET.into();
    let app = testing::spawn_with(cfg).await;
    let (_, alex) = testing::register(&app, "alex").await;
    let s = testing::create_server(&app, &alex, "Main").await;
    let (_, sam) = testing::register(&app, "sam").await;
    let r = testing::post_json(&app, &sam, &format!("/servers/{}/join", s.id), serde_json::json!({})).await;
    assert!(r.status().is_success(), "join server: {}", r.status());
    let api = Api::new(&format!("http://{}", app.addr));
    let lounge = api
        .channels(&alex, s.id)
        .await
        .unwrap()
        .into_iter()
        .find(|c| c.name.as_deref() == Some("Lounge"))
        .unwrap();
    let connect = |token: String| {
        let api = api.clone();
        async move {
            let vt = api.voice_token(&token, lounge.id).await.unwrap();
            Room::connect(&vt.url, &vt.token, RoomOptions::default()).await.unwrap()
        }
    };
    let (sender, _sender_events) = connect(alex.clone()).await;
    let (receiver, mut events) = connect(sam.clone()).await;

    let source = NativeAudioSource::new(AudioSourceOptions::default(), RATE, 1, 0);
    let track = LocalAudioTrack::create_audio_track("mic", RtcAudioSource::Native(source.clone()));
    sender
        .local_participant()
        .publish_track(LocalTrack::Audio(track), mic_publish_options())
        .await
        .unwrap();

    // the receiver's onsets, timed as frames come out of NetEq
    let (arrival_tx, mut arrivals) = mpsc::unbounded_channel::<Instant>();
    tokio::spawn(async move {
        while let Some(ev) = events.recv().await {
            if let RoomEvent::TrackSubscribed { track: RemoteTrack::Audio(t), .. } = ev {
                let tx = arrival_tx.clone();
                tokio::spawn(async move {
                    let mut stream = NativeAudioStream::new(t.rtc_track(), RATE as i32, 1);
                    let mut on = false;
                    while let Some(f) = stream.next().await {
                        let n = f.data.len().max(1) as f64;
                        let rms = (f.data.iter().map(|&s| (s as f64).powi(2)).sum::<f64>() / n)
                            .sqrt()
                            / 32768.0;
                        let now_on = rms > ON_RMS;
                        if now_on && !on {
                            let _ = tx.send(Instant::now());
                        }
                        on = now_on;
                    }
                });
            }
        }
    });
    tokio::time::sleep(Duration::from_secs(2)).await; // subscription start-up

    println!("| pattern | netem one-way ms | lag p50 | lag p90 | jb from lag | buffer | target | accel ms/5s | silent % | onsets |");
    println!("|---|---|---|---|---|---|---|---|---|---|");
    for pattern in [Pattern::Continuous, Pattern::Bursts] {
        run(pattern, &source, &receiver, &mut arrivals).await;
    }
    let _ = sender.close().await;
    let _ = receiver.close().await;
}
```

- [ ] **Step 6: Run the reproduction (four runs) and decide**

```bash
just dev-livekit
cargo test -p pulse-client --test neteq_probe -- --ignored --nocapture          # no netem
sudo tc qdisc add dev lo root netem delay 40ms 1ms                               # 80 ms RTT, ~1 ms jitter like the baseline (needs the owner's sudo)
NETEM_ONEWAY_MS=40 cargo test -p pulse-client --test neteq_probe -- --ignored --nocapture
sudo tc qdisc del dev lo root
```

Each run prints two table rows (Continuous, Bursts). Expected: ~29 onsets for Continuous and ~14 for
Bursts. If the counts are near 0, the threshold missed the tone, so check `ON_RMS` against the printed
`buffer` first. Decision rule, applied to the netem run (the baseline-like one):
- **artifact**: in `Continuous`, `jb from lag` is within 15 ms of `target` and `buffer − jb from lag ≥ 30` ms. The stat is not what anyone hears. If `buffer` is inflated **only** in `Bursts` (high `silent %`), the cause is DTX weighting: do Step 7.
- **standing-delay**: `jb from lag` is within 15 ms of `buffer` in `Continuous`. NetEq really holds that much. `accel` near 0 means it does not see it as excess (target too low or a clock issue), and steady `accel` means it is draining an excess that keeps coming back (the H4 pull-clock question).
- In between (both gaps 15-30 ms): run the Bursts/Continuous pair twice more and take the medians.

- [ ] **Step 7 (only if the verdict is "artifact" caused by DTX): report the jitter buffer over talking streams only**

Failing test first (`netstats.rs` tests):

```rust
    /// R5 verdict "DTX artifact": a mostly-silent stream's comfort-noise stretches don't count
    /// towards the jitter-buffer figures; talking streams do.
    #[test]
    fn silent_streams_do_not_skew_the_buffer() {
        let talking = inbound(1, |r| {
            r.inbound.jitter_buffer_delay = 4.8;
            r.inbound.jitter_buffer_target_delay = 4.8;
            r.inbound.jitter_buffer_emitted_count = 240_000;
            r.inbound.total_samples_received = 240_000;
        });
        let silent = inbound(2, |r| {
            r.inbound.jitter_buffer_delay = 96.0; // 400 ms per sample: SID packets waiting
            r.inbound.jitter_buffer_target_delay = 4.8;
            r.inbound.jitter_buffer_emitted_count = 240_000;
            r.inbound.total_samples_received = 240_000;
            r.inbound.silent_concealed_samples = 228_000;
        });
        let (s, _) = summarize(&[], &[talking, silent], &Totals::default());
        assert_eq!(s.buffer_ms.map(f64::round), Some(20.0));
        assert_eq!(s.silent_pct.map(f64::round), Some(48.0), "silence is still reported");
    }
```

Run `cargo test -p pulse-client --lib voice::netstats::tests::silent_streams_do_not_skew_the_buffer`.
It fails with `left: Some(210.0) right: Some(20.0)`. Then in `summarize` add a second accumulator,
`let mut talking = Inbound::default();`. In the `InboundRtp` arm, replace the `window.add(...)` line with:

```rust
                let d = now.since(prev.inbound.get(&r.stream.ssrc));
                window.add(&d);
                // DTX comfort noise weighs long waits into jitterBufferDelay that nobody hears
                if d.received > 0 && d.silent_concealed * 2 < d.received {
                    talking.add(&d);
                }
```

and compute `buffer_ms` / `target_ms` / `min_ms` from `talking` (same `per_sample` closure over
`talking.emitted`, `None` when `talking.emitted == 0`). `accel`, `decel`, `concealed` and `silent` stay on
`window`. Re-run `cargo test -p pulse-client --lib voice::netstats`. Expected: PASS.

- [ ] **Step 8: Write the note**

Create `docs/superpowers/research/opt-neteq-buffer.md` with exactly these sections:

```markdown
# NetEq "buffer" vs measured lag (R5)

Date: <YYYY-MM-DD>. Build: <short commit>. livekit-server v1.13 (dev, host network). libwebrtc 0.3.50.

Verdict: <artifact | standing-delay>

## How the path line computes it
Per inbound stream (SSRC), per 5 s window: Δ jitterBufferDelay / Δ jitterBufferEmittedCount (buffer),
Δ jitterBufferTargetDelay / Δ emitted (target). Our own queues are not in it (that's `recv`).

## Runs
<paste the four printed table rows under the header row the probe prints>

## Reading
<two to four sentences: which rule from R5 Step 6 matched, and the numbers that decided it>

## For the playout task
<only if standing-delay: "standing excess over target: N ms (pattern …), accel N ms per 5 s";
 otherwise: "nothing: the excess is a statistic, not audio">
```

- [ ] **Step 9: Commit**

```bash
git add client/src-tauri/src/voice/netstats.rs client/src-tauri/src/voice/mod.rs client/src-tauri/tests/neteq_probe.rs
git commit -m "fix(diagnostics): jitter-buffer stats per stream per window, plus accel/decel/silent; NetEq lag probe" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
git add docs/superpowers/research/opt-neteq-buffer.md
git commit -m "docs(research): NetEq buffer vs measured lag" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

(If Step 7 ran, include `netstats.rs` again in the second commit with message
`fix(diagnostics): jitter buffer over talking streams only (DTX comfort noise skewed it)`.)

---

---

## Phase 3: Mouth-to-ear

### Task 8 (L6): Mouth-to-ear (a): the voice gate decides on the undelayed signal

**Files:**
- Modify: `client/src-tauri/src/voice/denoise.rs:20-30` (trait), `:41-81` (`Rnnoise` remembers its cleaned level),
  `:83-140` (`DeepFilter`)
- Modify: `client/src-tauri/src/voice/gate.rs:20` (`PREROLL_BLOCKS` pub), `:94-115` (`process` takes the decision
  level; new `set_preroll`), tests `:130-147` (`feed` passes the block's RMS)
- Modify: `client/src-tauri/src/voice/processor.rs:54-111` (`BlockOut`, `MicChain::process`), `:171-236`
  (`StrongWorker` carries the level across its thread)
- Test: `client/src-tauri/src/voice/processor.rs`, `client/src-tauri/src/voice/denoise.rs` (`mod tests`)

**Interfaces:**
- Consumes: `Denoiser::process(&mut self, &mut [f32]) -> Option<f32>`; `VoiceGate::qualifies(level, prob)`.
- Produces: `Denoiser::gate_lead_blocks(&self) -> usize` (default 0) and `Denoiser::decision_level(&self) ->
  Option<f32>` (default `None` = use the raw input); `VoiceGate::process(&mut self, block, level: f32, prob:
  Option<f32>, now) -> bool`; `VoiceGate::set_preroll(&mut self, blocks: usize)`; `pub const PREROLL_BLOCKS`;
  `BlockOut { level, sent, in_level }` (`in_level`: RMS before cleaning and gain).
- Coordination: the stats task adding `ns_atten_db` / `gate_open_pct` must compute them from `BlockOut`:
  `ns_atten_db = 20·log10(in_level·gain / level)` over the window (gain from `ProcCfg`), `gate_open_pct` from
  `out.sent`. If that task already landed and reads `rms(block)` before `chain.process`, switch it to
  `out.in_level` here (one source of truth, no second RMS pass).

**Why:** voice H1. The gate tests the level of the *cleaned* block, which with Strong is 30 ms old (DFN: 2-frame
lookahead + 10 ms OLA), and then delays every sample 20 ms more for pre-roll. Deciding on the newest audio
(RNNoise's cleaned level for Strong, the raw input for Standard) makes the engine's own delay the pre-roll.
Expected: −20 ms one way with Strong (50 → 30 ms for denoise+gate), −10 ms with Standard (30 → 20 ms), Off
unchanged.

- [ ] **Step 1: Write the failing test**

`processor.rs` `mod tests` (add `use std::collections::VecDeque;`):

```rust
    fn rms(b: &[f32]) -> f32 {
        (b.iter().map(|v| v * v).sum::<f32>() / b.len() as f32).sqrt()
    }

    /// Stands in for an engine whose cleaned output lags its input by `delay` blocks and whose
    /// decision level runs `lead` blocks ahead of that output (Strong: 3 and 2; Standard: 1 and 1).
    struct Lagging {
        history: VecDeque<Vec<f32>>,
        delay: usize,
        lead: usize,
        decision: f32,
    }

    impl Lagging {
        fn new(delay: usize, lead: usize) -> Self {
            Self {
                history: (0..delay).map(|_| vec![0.0; FRAME]).collect(),
                delay,
                lead,
                decision: 0.0,
            }
        }
    }

    impl Denoiser for Lagging {
        fn process(&mut self, block: &mut [f32]) -> Option<f32> {
            self.history.push_back(block.to_vec()); // delay + 1 blocks, newest last
            let lag = self.delay - self.lead;
            self.decision = rms(&self.history[self.history.len() - 1 - lag]);
            let out = self.history.pop_front().expect("delay line");
            block.copy_from_slice(&out);
            Some(if self.decision > 0.05 { 0.95 } else { 0.0 })
        }
        fn gate_lead_blocks(&self) -> usize {
            self.lead
        }
        fn decision_level(&self) -> Option<f32> {
            // no lag: that's the raw input, which the chain measures itself
            (self.delay > self.lead).then_some(self.decision)
        }
    }

    /// How many blocks after a loud onset enters does it leave the chain at full level?
    fn onset_lag(delay: usize, lead: usize) -> usize {
        let mut c = MicChain::new(NsLevel::Off, true, 0.02, 1.0);
        c.set_denoiser(NsLevel::Standard, Box::new(Lagging::new(delay, lead)));
        let t0 = Instant::now();
        let mut k = 0u64;
        let mut step = |amp: f32| {
            let mut b = vec![amp; FRAME];
            c.process(&mut b, true, t0 + Duration::from_millis(k * 10));
            k += 1;
            rms(&b)
        };
        for _ in 0..100 {
            step(0.001); // learn the room
        }
        (0..20).position(|_| step(0.1) > 0.09).expect("the onset never got through")
    }

    /// The engine's lookahead is the gate's pre-roll: Strong's 30 ms, not 30 + 20.
    #[test]
    fn strong_onset_leaves_after_the_engine_delay_only() {
        assert_eq!(onset_lag(3, 2), 3, "Strong");
    }

    #[test]
    fn standard_onset_leaves_after_20ms_not_30() {
        assert_eq!(onset_lag(1, 1), 2, "Standard");
    }

    #[test]
    fn off_keeps_its_20ms_preroll() {
        assert_eq!(onset_lag(0, 0), 2, "Off");
    }

    #[test]
    fn block_out_reports_the_level_before_cleaning() {
        let mut c = MicChain::new(NsLevel::Off, false, 0.0, 2.0);
        let mut b = vec![0.1f32; FRAME];
        let out = c.process(&mut b, true, Instant::now());
        assert!((out.in_level - 0.1).abs() < 1e-3 && (out.level - 0.2).abs() < 1e-3);
    }
```

`denoise.rs` `mod tests` (guards the assumption the gate now relies on):

```rust
    /// Output-vs-input offset (in whole blocks) where the cleaned speech matches the input best.
    fn lag_blocks(d: &mut dyn Denoiser) -> usize {
        let input = speech();
        let (out, _) = run(d, &input);
        let n = out.len() - 5 * FRAME;
        let score = |lag: usize| {
            let (a, b) = (&input[..n], &out[lag..lag + n]);
            let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
            let norm = |x: &[f32]| x.iter().map(|v| v * v).sum::<f32>().sqrt();
            dot / (norm(a) * norm(b)).max(1e-9)
        };
        (0..=4)
            .max_by(|&x, &y| score(x * FRAME).total_cmp(&score(y * FRAME)))
            .unwrap()
    }

    /// The gate's pre-roll now comes from these delays (`gate_lead_blocks`): they must be real.
    #[test]
    fn engine_delays_match_what_the_gate_assumes() {
        assert_eq!(lag_blocks(&mut Rnnoise::new()), 1, "Standard: 10 ms");
        assert_eq!(lag_blocks(&mut DeepFilter::new().unwrap()), 3, "Strong: 30 ms");
        assert_eq!(Rnnoise::new().gate_lead_blocks(), 1);
        assert_eq!(DeepFilter::new().unwrap().gate_lead_blocks(), 2);
        assert_eq!(Passthrough.gate_lead_blocks(), 0);
    }
```

- [ ] **Step 2: Run test to verify it fails**

`cargo test -p pulse-client --lib voice::processor::tests::strong_onset voice::denoise::tests::engine_delays` →
compile errors `method gate_lead_blocks is not a member of trait Denoiser`, `no field in_level on BlockOut`.

- [ ] **Step 3: Write minimal implementation**

`denoise.rs`, trait `Denoiser` gains:

```rust
    /// How many 10 ms blocks `decision_level` runs ahead of the cleaned audio `process` returns.
    /// The gate decides on that level, so the engine's own delay becomes the gate's pre-roll.
    fn gate_lead_blocks(&self) -> usize {
        0
    }

    /// Level (RMS, unscaled) for the gate to decide on, from the newest input; `None` = the raw
    /// input's (the chain measures it).
    fn decision_level(&self) -> Option<f32> {
        None
    }
```

`Rnnoise`: add field `level: f32` (init `0.0`); at the end of `probability`, before returning `p`:

```rust
        let p = self.st.process_frame(&mut self.outb, &self.inb);
        // what RNNoise left of this input: 1 block behind it, 2 ahead of DeepFilterNet's output
        self.level = (self.outb.iter().map(|v| v * v).sum::<f32>() / FRAME as f32).sqrt() / I16_SCALE;
        p
```

`impl Denoiser for Rnnoise` adds (its output lags its input by one block; the raw input decides):

```rust
    fn gate_lead_blocks(&self) -> usize {
        1
    }
```

`impl Denoiser for DeepFilter` adds (DFN lags 3 blocks; RNNoise's cleaned level lags 1):

```rust
    fn gate_lead_blocks(&self) -> usize {
        2
    }

    fn decision_level(&self) -> Option<f32> {
        Some(self.vad.level)
    }
```

`gate.rs`: line 20 → `pub const PREROLL_BLOCKS: usize = 2;` (doc: "Pre-roll in front of the audio the gate
opens on: 20 ms, so first syllables survive. Part or all of it can come from the denoiser's own delay."). Add:

```rust
    /// How many blocks of delay the gate itself adds (the rest of the pre-roll is the denoiser's).
    pub fn set_preroll(&mut self, blocks: usize) {
        while self.delay.len() > blocks {
            self.delay.pop_front();
        }
        while self.delay.len() < blocks {
            self.delay.push_front(vec![0.0; FRAME]);
        }
    }
```

`process` (replaces `gate.rs:94-98`; the rest of the body is unchanged):

```rust
    /// Gate one block in place (input in, the block from `preroll` blocks ago out), deciding on
    /// `level`: the newest audio's, which may run ahead of `block`. Returns whether audio is sent.
    pub fn process(&mut self, block: &mut [f32], level: f32, prob: Option<f32>, now: Instant) -> bool {
        if self.qualifies(level, prob) {
            self.open_until = Some(now + HOLD);
        }
```

Gate tests: in `feed` (`gate.rs:139-145`) call `g.process(&mut b, rms(&b), prob, t0 + …)` (the test-local `rms`);
the existing tests then exercise the unchanged 2-block pre-roll.

`processor.rs`: `BlockOut` gains

```rust
    /// RMS of the block as it came in (before cleaning and gain): with `level`, what suppression removed.
    pub in_level: f32,
```

and `MicChain::process` becomes (`use super::gate::PREROLL_BLOCKS;`):

```rust
    pub fn process(&mut self, block: &mut [f32], mic_open: bool, now: Instant) -> BlockOut {
        let in_level = rms(block);
        let prob = self.denoiser.process(block);
        // Decide on the newest audio, not the cleaned block (which lags it): the engine's delay
        // then doubles as the gate's pre-roll instead of adding to it.
        let decide = self.denoiser.decision_level().unwrap_or(in_level) * self.gain;
        for v in block.iter_mut() {
            *v = soft_limit(*v * self.gain);
        }
        let level = rms(block);
        if !mic_open {
            self.gate.reset();
            block.fill(0.0);
            return BlockOut { level, sent: false, in_level };
        }
        self.gate
            .set_preroll(PREROLL_BLOCKS.saturating_sub(self.denoiser.gate_lead_blocks()));
        let sent = self.gate.process(block, decide, prob, now);
        BlockOut { level, sent, in_level }
    }
```

with a module-level `fn rms(b: &[f32]) -> f32 { (b.iter().map(|v| v * v).sum::<f32>() / b.len().max(1) as f32).sqrt() }`
(the test module's own `rms` from Step 1 can then be dropped in favour of `super::rms`).

`StrongWorker` carries the decision level across its thread: `rx: std_mpsc::Receiver<(Vec<f32>, Option<f32>,
Option<f32>)>`, a new field `level: Option<f32>` (init `None`, also in the `dead_strong_worker…` test literal), and

```rust
            Ok((out, p, level)) => {
                block.copy_from_slice(&out);
                self.level = level;
                p
            }
```

```rust
    fn gate_lead_blocks(&self) -> usize {
        2 // DeepFilter's, on its own thread
    }

    fn decision_level(&self) -> Option<f32> {
        self.level
    }
```

In `start_strong` the worker loop sends `(block, p, df.decision_level())`.

- [ ] **Step 4: Run test to verify it passes**

`cargo test -p pulse-client --lib voice::processor voice::gate voice::denoise` → all pass. If
`engine_delays_match_what_the_gate_assumes` reports a different lag, set `gate_lead_blocks` to
`measured_engine_lag − 1` for DeepFilter and `measured_rnnoise_lag` for Rnnoise (never guess; the test is the
source). Measurement: ear test with the gate on auto, "t-t-t" onsets with Strong and Standard: nothing clipped.
Acoustic clap test (voice research "measure first" #8): expect ~20 ms less with Strong.

- [ ] **Step 5: Commit**

`git add client/src-tauri/src/voice/denoise.rs client/src-tauri/src/voice/gate.rs client/src-tauri/src/voice/processor.rs`
`git commit -m "perf(voice): gate decides on the newest audio; the denoiser's delay is the pre-roll (-20 ms Strong)"`

---

### Task 9 (L7b): Mouth-to-ear (b): drift-compensated playout with a ~20 ms queue

**Files:**
- Modify: `client/src-tauri/src/voice/mixer.rs:282-287` (constants), `:311-437` (`Mixer`, `Peer`, `mix_into`), tests
  `:439-613`
- Modify: `client/src-tauri/src/voice/mod.rs:462-466` (path line gains `adjust`)
- Test: `client/src-tauri/src/voice/mixer.rs` (`mod tests`)

**Interfaces:**
- Consumes: `skip(&mut VecDeque<i16>, n)` (crossfaded cut, `mixer.rs:291-309`).
- Produces: `Mixer::take_adjusted_samples(&mut self) -> u64`; constants `TARGET_MS = 10` (lowest fill),
  `WINDOW_MS = 250`, `MAX_ADJUST_PER_MILLE = 5`, `CUT_ABOVE_MS = 80`, `QUIET = 328`. `Mixer::new`,
  `push`, `mix_into` keep their signatures (L7a then moves them to the split mixer).

**Why:** voice H2/H4. The queue settles wherever start-up leaves it and is only trimmed above 40 + 15 ms; when the
speakers' clock runs faster than WebRTC's 10 ms timer the queue drains and then underruns a few samples at a time,
forever (tiny zero-insert clicks). Design: per peer, judge the lowest fill (just after each speaker pull) every
250 ms against a 10 ms target (margin for the sender's timer jitter, so the queue averages ~20 ms); correct the
difference by dropping or repeating single samples near zero crossings (|s| < 1 % FS, inaudible), at most 0.5 %
(60 samples per window); cut (crossfaded) only when > 80 ms over; after an underrun, refill to 20 ms before
playing on (one gap, not a dribble). Expected: −15-35 ms on a standing queue (40-55 → ~20 ms), no drift clicks.
Note the 2026-10-15 Windows baseline already showed recv p50 20 ms with 10 ms device frames, so the typical
Windows gain is the max/standing case and the clicks; Linux ALSA's 215-273 ms is device buffering, not this queue.

- [ ] **Step 1: Write the failing test**

Add to `mixer.rs` `mod tests`:

```rust
    use std::f64::consts::TAU;

    fn tone(i: usize) -> i16 {
        (10_000.0 * (i as f64 * 437.0 * TAU / 48_000.0).sin()) as i16
    }

    /// One sender, one speaker: per 10 ms pulled, the sender delivers 480 × (1 + ppm/10⁶) samples
    /// (fraction carried). Returns what played, as i16-scaled values.
    fn drifting(m: &mut Mixer, secs: usize, ppm: f64, next: &mut usize) -> Vec<i32> {
        let (mut owed, mut out, mut played) = (0.0f64, vec![0.0f32; 480], Vec::new());
        for _ in 0..secs * 100 {
            owed += 480.0 * (1.0 + ppm / 1e6);
            let n = owed as usize;
            owed -= n as f64;
            let chunk: Vec<i16> = (*next..*next + n).map(tone).collect();
            *next += n;
            m.push("a", &chunk);
            m.mix_into(&mut out, 1, 1.0);
            played.extend(out.iter().map(|v| (v * i16::MAX as f32) as i32));
        }
        played
    }

    fn max_step(p: &[i32]) -> i32 {
        p.windows(2).map(|w| (w[1] - w[0]).abs()).max().unwrap_or(0)
    }

    /// The sender's clock runs fast (10× a bad real-world drift, so a minute shows it): the
    /// queue must stay near the target, eased down without clicks, never trimmed in big cuts.
    #[test]
    fn a_fast_sender_is_held_near_the_target() {
        let mut m = Mixer::new(48_000, 200);
        let mut next = 0;
        let played = drifting(&mut m, 60, 1000.0, &mut next);
        assert!(m.buffered("a") <= (TARGET_MS + 12) * 48, "queue grew to {} samples", m.buffered("a"));
        assert_eq!(m.take_underrun_samples(), 0);
        assert!(max_step(&played) < 1500, "a click: step of {}", max_step(&played));
        assert!(m.take_adjusted_samples() > 0, "drift was corrected sample by sample");
    }

    /// The sender's clock runs slow: once settled, the speakers must never run dry (today they
    /// do, a few samples at a time, forever).
    #[test]
    fn a_slow_sender_never_underruns_once_settled() {
        let mut m = Mixer::new(48_000, 200);
        let mut next = 0;
        drifting(&mut m, 10, -1000.0, &mut next);
        m.take_underrun_samples();
        let played = drifting(&mut m, 50, -1000.0, &mut next);
        assert_eq!(m.take_underrun_samples(), 0, "ran dry while settled");
        assert!(max_step(&played) < 1500, "a click: step of {}", max_step(&played));
    }

    /// After running dry (the sender stalled), playback resumes with a cushion, not at zero
    /// margin where the next late frame is another gap.
    #[test]
    fn after_an_underrun_playback_resumes_with_a_cushion() {
        let mut m = Mixer::new(48_000, 200);
        let mut next = 0;
        drifting(&mut m, 1, 0.0, &mut next);
        let mut out = vec![0.0f32; 480];
        for _ in 0..5 {
            m.mix_into(&mut out, 1, 1.0); // 50 ms with nothing arriving
        }
        assert!(m.take_underrun_samples() > 0);
        drifting(&mut m, 1, 0.0, &mut next);
        assert!(
            m.buffered("a") >= TARGET_MS * 48,
            "resumed with {} samples of margin",
            m.buffered("a")
        );
    }

    /// A queue a little over the target (start-up) is eased down, not cut.
    #[test]
    fn a_small_excess_is_eased_out_without_a_cut() {
        let mut m = Mixer::new(48_000, 200);
        let pre: Vec<i16> = (0..48 * 45).map(tone).collect(); // 45 ms
        m.push("a", &pre);
        let mut next = pre.len();
        let played = drifting(&mut m, 3, 0.0, &mut next);
        let left = m.buffered("a");
        assert!(left < 48 * 45 && left > TARGET_MS * 48, "{left} queued");
        assert!(max_step(&played) < 1500, "a click: step of {}", max_step(&played));
    }
```

Replace the three tests whose premise changes:
- `a_standing_backlog_drains_to_the_target` → keep, with `assert!(m.buffered("a") <= TARGET_MS * 48 + 480, …)`.
- `a_small_excess_from_clock_drift_is_left_alone` → delete (superseded by `a_small_excess_is_eased_out_without_a_cut`).
- `a_short_queue_is_left_alone` → rename `a_queue_at_the_target_is_left_alone`, push `&[100; 480]` (10 ms), assert
  `m.buffered("a") == 480` after `steady(&mut m, 3000)`.

- [ ] **Step 2: Run test to verify it fails**

`cargo test -p pulse-client --lib voice::mixer` → compile error `cannot find value TARGET_MS` / `no method
take_adjusted_samples`. With those two stubbed on HEAD: `a_fast_sender…` fails (`queue grew to ~2600 samples`),
`a_slow_sender…` fails (`ran dry while settled`, left ≈ 2400), `after_an_underrun…` fails (`resumed with 0 samples`).

- [ ] **Step 3: Write minimal implementation**

`mixer.rs:282-287` replaced by:

```rust
/// Each peer's queue is steered so its lowest point (just after the speakers pull) sits here:
/// margin for the sender's 10 ms timer jitter. It then averages about this plus one 10 ms frame
/// (~20 ms) instead of settling wherever start-up left it (anything up to 55 ms before).
pub const TARGET_MS: usize = 10;
/// How often each peer's fill is judged and its correction budget renewed.
const WINDOW_MS: usize = 250;
/// Most samples dropped or repeated per window: 0.5 % of what plays, inaudible on speech.
const MAX_ADJUST_PER_MILLE: usize = 5;
/// Further over the target than this (an output reopen, a stall): cut straight back, crossfaded.
const CUT_ABOVE_MS: usize = 80;
/// A sample this quiet (1 % of full scale, near a zero crossing) is where one is dropped or
/// repeated: the step can't be heard.
const QUIET: u16 = 328;
```

`Mixer` fields: replace `target, slack` with

```rust
    /// Lowest fill wanted (samples), and what a dry peer refills to before playing on.
    target: usize,
    refill: usize,
    window: usize,
    /// Samples a peer may drop or repeat per window.
    budget: usize,
    cut_above: usize,
    /// Samples dropped or repeated so far (diagnostics: how hard drift is being corrected).
    adjusted: u64,
```

initialised in `new` as

```rust
            target: sample_rate as usize * TARGET_MS / 1000,
            refill: sample_rate as usize * TARGET_MS / 1000 + sample_rate as usize / 100,
            window: sample_rate as usize * WINDOW_MS / 1000,
            budget: sample_rate as usize * WINDOW_MS / 1000 * MAX_ADJUST_PER_MILLE / 1000,
            cut_above: sample_rate as usize * CUT_ABOVE_MS / 1000,
            adjusted: 0,
```

`Peer` gains `adjust: i32` (> 0 drop, < 0 repeat, this window), `held: bool`, `rebuffering: bool` (all
`0/false` in `peer()`), and:

```rust
impl Peer {
    /// The next sample to play, steering towards the target: while a correction is due, drop or
    /// repeat one quiet sample. `(sample, adjusted)`; `None` = ran dry.
    fn next(&mut self) -> Option<(i16, bool)> {
        let s = *self.queue.front()?;
        let quiet = s.unsigned_abs() < QUIET;
        if self.adjust < 0 && quiet && !self.held {
            self.adjust += 1;
            self.held = true; // play it again next time (once)
            return Some((s, true));
        }
        self.held = false;
        self.queue.pop_front();
        if self.adjust > 0 && quiet && !self.queue.is_empty() {
            self.adjust -= 1;
            self.queue.pop_front(); // and skip the one after it
            return Some((s, true));
        }
        Some((s, false))
    }
}
```

`mix_into` (replaces `mixer.rs:411-436`):

```rust
    /// Fill interleaved `out` (`channels` per frame), one sample per peer per frame.
    pub fn mix_into(&mut self, out: &mut [f32], channels: usize, master: f32) {
        for p in self.peers.values_mut() {
            if p.rebuffering && p.queue.len() >= self.refill {
                p.rebuffering = false;
            }
        }
        for frame in out.chunks_mut(channels.max(1)) {
            let mut acc = 0.0f32;
            for p in self.peers.values_mut() {
                if p.rebuffering {
                    self.underrun += 1;
                    continue;
                }
                match p.next() {
                    Some((s, adjusted)) => {
                        acc += s as f32 / i16::MAX as f32 * p.gain;
                        self.adjusted += adjusted as u64;
                    }
                    None => {
                        // ran dry: refill before playing on (one gap, not a dribble of clicks)
                        self.underrun += 1;
                        p.rebuffering = true;
                    }
                }
            }
            frame.fill(soft_limit(acc * master));
        }
        for p in self.peers.values_mut() {
            p.low = p.low.min(p.queue.len());
        }
        self.pulled += out.len() / channels.max(1);
        if self.pulled >= self.window {
            self.pulled = 0;
            let budget = self.budget as i64;
            for p in self.peers.values_mut() {
                if p.low != usize::MAX && !p.rebuffering {
                    let over = p.low as i64 - self.target as i64;
                    if over > self.cut_above as i64 {
                        skip(&mut p.queue, over as usize);
                        p.adjust = 0;
                    } else {
                        p.adjust = over.clamp(-budget, budget) as i32;
                    }
                }
                p.low = usize::MAX;
            }
        }
    }

    /// Samples dropped or repeated (drift correction) since the last call.
    pub fn take_adjusted_samples(&mut self) -> u64 {
        std::mem::take(&mut self.adjusted)
    }
```

`mod.rs:464-466` (path line): add `let adjust = mixer.lock().unwrap().take_adjusted_samples();` next to the
underrun take and log it: `tracing::info!(%line, send_ms, adjust, %ours, "voice: path");`.

- [ ] **Step 4: Run test to verify it passes**

`cargo test -p pulse-client --lib voice::mixer` → all pass (including the unchanged `trimming_a_backlog_does_not_click`,
`underruns_are_counted_per_peer_sample_missed`, `cap_drops_oldest`). Measurement: a 30-minute two-client call
(Windows + Linux); in `voice: path`, `recv a/b/c ms` max should stay ≤ ~30 ms after the first window (baseline
max up to 55), `underrun` 0 ms except real network gaps, `adjust` small (≤ ~50 samples per 5 s at 100 ppm).

- [ ] **Step 5: Commit**

`git add client/src-tauri/src/voice/mixer.rs client/src-tauri/src/voice/mod.rs`
`git commit -m "perf(voice): drift-compensated playout around a ~20 ms queue; refill after an underrun"`

---

### Task 10 (L8): Mouth-to-ear (c): mic frames go to WebRTC straight from the processor thread

**Files:**
- Modify: `client/src-tauri/src/voice/processor.rs:252-263` (`spawn` without a channel), `:278-408` (`run`: sink,
  gap fill), tests `:492-605`
- Modify: `client/src-tauri/src/voice/devices.rs:272-331` (`Shared.mic_sink`, `Shared.mic_queue`, `set_mic_sink`)
- Modify: `client/src-tauri/src/voice/mod.rs:42-47` (drop `SEND_SLACK_CHUNKS`), `:386-390`, `:408-414`,
  `:440-543` (delete the tokio sender task), `:910-915` (`send_queue_ms`)
- Modify: `client/src-tauri/src/voice/mictest.rs:52-102` (loopback becomes a sink)
- Test: `client/src-tauri/src/voice/processor.rs` (`mod tests`)

**Interfaces:**
- Consumes: `NativeAudioSource::capture_frame(&self, &AudioFrame<'_>) -> impl Future<Output = Result<(), RtcError>>`,
  synchronous on the queue-0 fast path (libwebrtc 0.3.50 `native/audio_source.rs:100-147`);
  `backlog::GapFill`; `AudioFrame { data: Cow<[i16]>, sample_rate, num_channels, samples_per_channel }`.
- Produces: `pub trait FrameSink: Send { fn send(&mut self, frame: &[i16]) -> bool; }`;
  `pub struct LiveKitSink(NativeAudioSource)` with `LiveKitSink::new`; `Shared::set_mic_sink(&self,
  Option<Box<dyn FrameSink>>)`; `Shared.mic_queue: backlog::QueueGauge`; `processor::spawn(shared: Arc<Shared>)
  -> (RawTx, JoinHandle<()>)`.

**Why:** voice M4. Each 10 ms block goes `pulse-mic` → unbounded tokio channel → async task → `ToInternal`
(always the 48 kHz copy branch) → `ready` buffer → `collect` → `capture_frame`: one thread hop through the Tauri
runtime and 3 allocations + copies per 10 ms. Under runtime load (IPC bursts, device probes, report gathering) the
hop costs ms and the 10 ms gap-fill interval ticks at timer granularity. Expected: 0-5 ms less send jitter, the
`send_ms` backlog gone (the processor's 60 ms MAX_LAG still bounds it), simpler code.

- [ ] **Step 1: Write the failing test**

`processor.rs` `mod tests` (uses `shared_with` from the existing tests):

```rust
    type Seen = Arc<Mutex<Vec<(usize, Option<String>)>>>;

    /// Records each frame's length and the thread it arrived on.
    struct Collect(Seen);

    impl FrameSink for Collect {
        fn send(&mut self, frame: &[i16]) -> bool {
            let thread = std::thread::current().name().map(String::from);
            self.0.lock().unwrap().push((frame.len(), thread));
            true
        }
    }

    fn collecting(sh: &Arc<Shared>) -> Seen {
        let seen: Seen = Default::default();
        sh.set_mic_sink(Some(Box::new(Collect(seen.clone()))));
        seen
    }

    fn wait_frames(seen: &Seen, n: usize) -> usize {
        let deadline = Instant::now() + Duration::from_secs(5);
        while seen.lock().unwrap().len() < n && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(2));
        }
        seen.lock().unwrap().len()
    }

    fn off() -> Arc<Shared> {
        shared_with(AudioConfig {
            noise_suppression: NsLevel::Off,
            ..Default::default()
        })
    }

    #[test]
    fn frames_go_straight_to_the_sink_on_the_processor_thread() {
        let sh = off();
        let seen = collecting(&sh);
        let (raw_tx, _h) = spawn(sh.clone());
        for _ in 0..10 {
            raw_tx.send((48_000, vec![0.0; 480], Instant::now())).unwrap();
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(wait_frames(&seen, 10) >= 10);
        let seen = seen.lock().unwrap();
        assert!(
            seen.iter().all(|(n, t)| *n == 480 && t.as_deref() == Some("pulse-mic")),
            "{seen:?}"
        );
    }

    /// Before the room is connected there's nowhere to send: frames are dropped, never queued
    /// (whatever piled up would be sent late for the rest of the call).
    #[test]
    fn without_a_room_frames_are_dropped_not_queued() {
        let sh = off();
        let (raw_tx, _h) = spawn(sh.clone());
        for _ in 0..30 {
            raw_tx.send((48_000, vec![0.0; 480], Instant::now())).unwrap();
        }
        std::thread::sleep(Duration::from_millis(100));
        let seen = collecting(&sh);
        raw_tx.send((48_000, vec![0.0; 480], Instant::now())).unwrap();
        wait_frames(&seen, 1);
        std::thread::sleep(Duration::from_millis(30));
        assert_eq!(seen.lock().unwrap().len(), 1, "only the frame after the room connected");
    }

    /// The mic stalls (a device reopen): the stream to WebRTC stays continuous with silence.
    #[test]
    fn a_stalled_mic_is_covered_with_silence() {
        let sh = off();
        let seen = collecting(&sh);
        let (raw_tx, _h) = spawn(sh.clone());
        for _ in 0..5 {
            raw_tx.send((48_000, vec![0.0; 480], Instant::now())).unwrap();
        }
        std::thread::sleep(Duration::from_millis(250));
        assert!(wait_frames(&seen, 5 + 10) >= 15, "{} frames", seen.lock().unwrap().len());
        assert!(sh.stats.silence_fill.load(std::sync::atomic::Ordering::Relaxed) >= 10);
        drop(raw_tx);
    }
```

Rewrite the existing channel-based tests on the sink: `processor_emits_exact_48k_frames`,
`processor_handles_rate_change` (replace `recv_n(&mut mic_rx, n)` with `wait_frames(&seen, n)` and check the
recorded lengths are all 480), `bursty_device_callbacks_lose_nothing` (assert `wait_frames(&seen, 30) >= 30` and
`sh.stats.lag_drops == 0`: silence fill between bursts is expected now), `processor_exits_when_senders_drop`
(`spawn(shared_with(AudioConfig::default()))`). Delete `recv_n`.

- [ ] **Step 2: Run test to verify it fails**

`cargo test -p pulse-client --lib voice::processor` → compile errors `cannot find trait FrameSink`, `no method
set_mic_sink on Arc<Shared>`, `spawn takes 2 arguments but 1 was supplied`.

- [ ] **Step 3: Write minimal implementation**

`processor.rs` (new, above `spawn`):

```rust
use livekit::webrtc::audio_frame::AudioFrame;
use livekit::webrtc::audio_source::native::NativeAudioSource;

use super::backlog::GapFill;

/// Where finished 10 ms mic frames (48 kHz mono i16) go, called on the processor thread.
pub trait FrameSink: Send {
    /// Hand over one frame; false if it was refused (counted as a send drop).
    fn send(&mut self, frame: &[i16]) -> bool;
}

/// The room's mic track. The fast path (queue 0) is synchronous: each frame goes straight to
/// WebRTC's encoder from here, with no runtime hop and no copy beyond the i16 conversion.
pub struct LiveKitSink(NativeAudioSource);

impl LiveKitSink {
    pub fn new(source: NativeAudioSource) -> Self {
        Self(source)
    }
}

impl FrameSink for LiveKitSink {
    fn send(&mut self, frame: &[i16]) -> bool {
        use futures::FutureExt;
        let f = AudioFrame {
            data: std::borrow::Cow::Borrowed(frame),
            sample_rate: INTERNAL_RATE,
            num_channels: 1,
            samples_per_channel: frame.len() as u32,
        };
        // queue 0 never awaits: `None` would mean a buffered source, which we never build
        matches!(self.0.capture_frame(&f).now_or_never(), Some(Ok(())))
    }
}

/// How often the processor looks for a stalled mic when no block arrives.
const GAP_CHECK: Duration = Duration::from_millis(10);

/// Hand `frame` to the room, if there is one. Between rooms there's nowhere to send: the frame
/// is dropped, and the gap clock restarts so a new room doesn't get a burst of "missed" silence.
fn deliver(shared: &Shared, gap: &mut GapFill, frame: &[i16]) {
    let now = Instant::now();
    let sent = shared.mic_sink.lock().unwrap().as_mut().map(|s| s.send(frame));
    match sent {
        Some(false) => {
            shared.stats.send_drops.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        Some(true) | None => gap.sent(now),
    }
}

/// The mic delivered nothing for a while (stall, device reopen): keep the room's stream
/// continuous with silent frames, or listeners' jitter buffers grow to cover the hole.
fn fill_gap(shared: &Shared, gap: &mut GapFill, silence: &[i16]) {
    let now = Instant::now();
    let mut sink = shared.mic_sink.lock().unwrap();
    let Some(s) = sink.as_mut() else {
        gap.sent(now);
        return;
    };
    let due = gap.silence_due(now);
    for _ in 0..due {
        s.send(silence);
    }
    if due > 0 {
        gap.silence_sent(due);
        shared
            .stats
            .silence_fill
            .fetch_add(due as u64, std::sync::atomic::Ordering::Relaxed);
    }
}
```

`spawn` (replaces `:252-263`):

```rust
/// Start the mic processor. It runs until every returned sender is dropped; its frames go to
/// `shared`'s mic sink (set per room).
pub fn spawn(shared: Arc<Shared>) -> (RawTx, std::thread::JoinHandle<()>) {
    let (tx, rx) = std_mpsc::channel::<RawBlock>();
    let h = std::thread::Builder::new()
        .name("pulse-mic".into())
        .spawn(move || run(shared, rx))
        .expect("spawn mic processor");
    (tx, h)
}
```

`run(shared, rx)`: drop the `mic_tx` parameter; before the loop add
`let (mut gap, silence) = (GapFill::new(Instant::now()), vec![0i16; FRAME]);`; replace the loop head
`while let Ok(first) = rx.recv() {` with

```rust
    loop {
        let first = match rx.recv_timeout(GAP_CHECK) {
            Ok(b) => b,
            Err(std_mpsc::RecvTimeoutError::Timeout) => {
                fill_gap(&shared, &mut gap, &silence);
                continue;
            }
            Err(std_mpsc::RecvTimeoutError::Disconnected) => return, // devices closed for good
        };
```

after `trim_backlog`: `shared.mic_queue.record(queue.len() as u32 * 10);` (blocks waiting = ms behind); and the
tail of the per-block loop (replaces `:402-405`):

```rust
            for (d, v) in i16buf.iter_mut().zip(block.iter()) {
                *d = to_i16(*v);
            }
            deliver(&shared, &mut gap, &i16buf);
```

`devices.rs` `Shared` gains (and `Shared::new` inits `mic_sink: Mutex::new(None), mic_queue: Default::default()`):

```rust
    /// Where the processor's frames go: the current room's mic track (`None` between rooms).
    pub(crate) mic_sink: Mutex<Option<Box<dyn super::processor::FrameSink>>>,
    /// Mic audio waiting to be processed (ms), worst per stats window: the send-side delay we own.
    pub mic_queue: super::backlog::QueueGauge,
```

```rust
    pub fn set_mic_sink(&self, sink: Option<Box<dyn super::processor::FrameSink>>) {
        *self.mic_sink.lock().unwrap() = sink;
    }
```

`mod.rs`: delete `SEND_SLACK_CHUNKS` (`:44-47`), the `mic_tx/mic_rx` channel (`:388`), the `send_queue` gauge
(`:440`, field `Session.send_queue`) and the whole "mic → LiveKit" task (`:471-543`). `:390` becomes
`let (raw_tx, _processor) = processor::spawn(shared.clone());`. After the track is created (`:413-414`):

```rust
        // from now on the mic processor hands each 10 ms frame straight to WebRTC
        shared.set_mic_sink(Some(Box::new(processor::LiveKitSink::new(source.clone()))));
```

In the stats task replace `send_queue.take_worst()` with `shared.mic_queue.take_worst()`; `send_queue_ms` returns
`s.shared.mic_queue.current()`. In `leave_locked` (before `room.close()`) and in the `Disconnected` arm:
`shared.set_mic_sink(None);` (the `Disconnected` task gets a `shared` clone). Remove the now-unused imports
(`AudioFrame`, `mpsc`, `MicChunk` from the `use devices::…` line).

`mictest.rs`: replace the channel + loopback task (`:61-75`) with a sink (the processor ends with its senders as
before):

```rust
/// The mic test's "speaker": your processed mic, delayed, into the local mixer.
struct Loopback {
    delay: DelayLine,
    mixer: Arc<Mutex<Mixer>>,
}

impl super::processor::FrameSink for Loopback {
    fn send(&mut self, frame: &[i16]) -> bool {
        let out = self.delay.process(frame);
        self.mixer.lock().unwrap().push("self", &out);
        true
    }
}
```

```rust
        shared.set_mic_sink(Some(Box::new(Loopback {
            delay: DelayLine::new((INTERNAL_RATE * DELAY_MS / 1000) as usize),
            mixer: mixer.clone(),
        })));
        let (raw_tx, _processor) = super::processor::spawn(shared.clone());
```

and `running` holds `vec![levels]` only. (`resampler::ToInternal` and `devices::MicChunk` stay `pub` and unused by
the call path; leave them, the mic-rate edge is in the processor.)

- [ ] **Step 4: Run test to verify it passes**

`cargo test -p pulse-client --lib voice` → all pass. `just voice-it` (dev LiveKit) → `one_client_hears_another_at_real_time`
(0.9-1.1× real time) and `mic_audio_is_not_sent_late_after_joining` (now reads the processor queue) pass.
Measurement: `voice: path send_ms` drops to 0-10 (baseline: worst send backlog per window); `fill` and `send`
drop counters unchanged in a quiet call.

- [ ] **Step 5: Commit**

`git add client/src-tauri/src/voice/processor.rs client/src-tauri/src/voice/devices.rs client/src-tauri/src/voice/mod.rs client/src-tauri/src/voice/mictest.rs`
`git commit -m "perf(voice): send mic frames to WebRTC straight from the processor thread"`

---

### Task 11 (L7a): The speaker callback never blocks on a lock a network task holds (A7)

**Files:**
- Modify: `client/src-tauri/Cargo.toml` (`rtrb = "0.3"`, already in `Cargo.lock` as 0.3.5 via libwebrtc)
- Modify: `client/src-tauri/src/voice/mixer.rs` (split into `MixerInput` + `Mixer`; all tests)
- Modify: `client/src-tauri/src/voice/devices.rs:272-331` (`Shared`: `mixer`, `mix_in`, `playout_on`, `mix_busy`;
  `Shared::new(cfg, controls, peer_buffer_ms)`), `:368-415` (`on_output`), tests `:1125-1190`
- Modify: `client/src-tauri/src/voice/mod.rs` (every `mixer.lock()` → `shared.mix_in.…`), `mictest.rs`, `processor.rs`
  tests (`shared_with`)
- Test: `client/src-tauri/src/voice/devices.rs`, `client/src-tauri/src/voice/mixer.rs` (`mod tests`)
- cpal: touches `devices.rs` around the output callback (`on_output`, called from `build_output`). The cpal-upgrade
  section changes `build_output`/`open_half`; `on_output`'s signature is unchanged here, so rebase is mechanical.

**Interfaces:**
- Consumes: `rtrb::RingBuffer::<i16>::new(n) -> (Producer, Consumer)`, `Producer::write_chunk_uninit(n)` +
  `WriteChunkUninit::fill_from_iter`, `Consumer::read_chunk(n)` + `ReadChunk::{as_slices, commit_all}`,
  `{Producer,Consumer}::slots()`.
- Produces: `pub fn mixer(sample_rate: u32, cap_ms: u32) -> (MixerInput, Mixer)`;
  `MixerInput::{push, set_gain, remove, clear, take_peer_levels, take_underrun_samples, take_adjusted_samples,
  pushed_total, max_buffered}` (all `&self`); `Mixer::{mix_into, buffered}` (speaker side, `&mut self`);
  `Shared.mixer: Mutex<Mixer>` (only the speaker callback takes it, with `try_lock`),
  `Shared.mix_in: Arc<MixerInput>`, `Shared::take_mix_busy(&self) -> u64`.

**Why:** voice M3 and spec A7. `on_output` takes 4 mutexes per callback; the mixer one is also taken by every
receive task's `push` (a tokio worker that can be preempted holding it), the 100 ms levels task and the 5 s stats
task; `controls` and `spk_meter` by UI-driven tasks. A preempted holder makes the real-time callback wait a
scheduler quantum (up to ~15 ms on Windows): an underrun click, worse once the audio threads run at raised
priority (cpal ≥ 0.16, other section). Design: per-peer lock-free SPSC rings carry samples; peer add/gain/remove
go through a command list the callback drains with `try_lock`; meters/fill/counters are atomics or live on the
network side; `controls` and the speaker meter are `try_lock` with a cached fallback. Echo-reference
(`echo_ref`) stays a `Mutex`: only the mic processor takes it, for a microsecond drain. No ms gain; removes a
class of rare clicks under load.

- [ ] **Step 1: Write the failing test**

`devices.rs` `mod tests`:

```rust
    /// The speaker callback is real-time: a receive task preempted mid-push, a mute toggle in
    /// progress, or the levels task reading the meter must never make it wait.
    #[test]
    fn playback_never_waits_on_the_network_side() {
        let s = Shared::new(&AudioConfig::default(), Default::default(), 200);
        s.mix_in.push("a", &[100; 480]);
        let held_peers = s.mix_in.peers.lock().unwrap();
        let held_controls = s.controls.lock().unwrap();
        let held_meter = s.spk_meter.lock().unwrap();
        let s2 = s.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut out = vec![0f32; 960]; // 10 ms stereo at 48 kHz
            let mut playout = super::super::playout::Playout::new(48_000);
            s2.on_output(&mut out, 2, 48_000, &mut Vec::new(), &mut playout);
            let _ = tx.send(());
        });
        let done = rx.recv_timeout(Duration::from_millis(500)).is_ok();
        drop((held_peers, held_controls, held_meter));
        assert!(done, "the speaker callback blocked on a lock the network side holds");
    }
```

`mixer.rs` `mod tests`, new:

```rust
    /// Receive tasks and the speakers share no lock: pushes from another thread arrive.
    #[test]
    fn pushes_from_another_thread_reach_the_speakers() {
        let (input, mut m) = mixer(48_000, 200);
        let input = std::sync::Arc::new(input);
        let i2 = input.clone();
        std::thread::spawn(move || i2.push("a", &[8192; 480])).join().unwrap();
        let mut out = vec![0.0f32; 480];
        m.mix_into(&mut out, 1, 1.0);
        assert!((out[0] - 0.25).abs() < 1e-3);
        assert_eq!(input.max_buffered(), 0, "fill is published after each pull");
        assert_eq!(input.pushed_total(), 480);
    }

    #[test]
    fn clear_forgets_the_last_rooms_peers() {
        let (input, mut m) = mixer(48_000, 200);
        input.push("a", &[100; 960]);
        input.clear();
        assert_eq!(m.buffered("a"), 0);
    }
```

Convert every existing mixer test to the split API: `let (input, mut m) = mixer(48_000, 200);`, pushes and
`set_gain`/`remove`/`take_peer_levels`/`take_underrun_samples`/`take_adjusted_samples` on `input`, `mix_into` and
`buffered` on `m`; `steady(&input, &mut m, ms)`, `drifting(&input, &mut m, …)`. `cap_drops_oldest` becomes:

```rust
    #[test]
    fn overflow_keeps_the_newest_audio_bounded() {
        let (input, mut m) = mixer(1000, 10); // cap = 10 samples
        let samples: Vec<i16> = (0..25).collect();
        input.push("a", &samples);
        assert_eq!(m.buffered("a"), 10);
        let mut out = [0.0f32; 10];
        m.mix_into(&mut out, 1, 1.0);
        // the newest 10 are kept, faded in from what was cut: last = 0.1 x 14 + 0.9 x 24
        assert!((out[9] * i16::MAX as f32 - 23.0).abs() < 1.0, "{}", out[9] * i16::MAX as f32);
    }
```

- [ ] **Step 2: Run test to verify it fails**

`cargo test -p pulse-client --lib voice::devices::tests::playback_never_waits voice::mixer` → compile errors
(`Shared::new` takes 3 args of a different type, `no field mix_in`, `cannot find function mixer`). On HEAD with the
test adapted to lock `s.mixer` instead of `mix_in.peers`, it fails with "the speaker callback blocked…".

- [ ] **Step 3: Write minimal implementation**

`Cargo.toml` `[dependencies]`: `rtrb = "0.3"`.

`mixer.rs`: keep `percent_to_gain`, `soft_limit`, `skip`, the L7b constants and `Peer::next`. Replace `Mixer::new`,
`Mixer`, `Peer` and their impls with:

```rust
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering::Relaxed};
use std::sync::{Arc, Mutex};

use super::meter::Meter;

/// Network side → speaker side changes, applied by the speakers at their next pull.
enum Cmd {
    Add { id: String, ring: rtrb::Consumer<i16>, filled: Arc<AtomicUsize> },
    Gain { id: String, gain: f32 },
    Remove { id: String },
    Clear,
}

/// A mixer in two halves that share no lock the speakers wait on: `MixerInput` for the
/// receive tasks and UI, `Mixer` for the speaker callback only.
pub fn mixer(sample_rate: u32, cap_ms: u32) -> (MixerInput, Mixer) {
    let rate = sample_rate as usize;
    let cap = (rate * cap_ms as usize / 1000).max(1);
    let cmds = Arc::new(Mutex::new(Vec::new()));
    let (underrun, adjusted) = (Arc::new(AtomicU64::new(0)), Arc::new(AtomicU64::new(0)));
    let input = MixerInput {
        cmds: cmds.clone(),
        peers: Mutex::new(HashMap::new()),
        cap,
        pushed: AtomicU64::new(0),
        underrun: underrun.clone(),
        adjusted: adjusted.clone(),
    };
    let speakers = Mixer {
        cmds,
        cap,
        peers: HashMap::with_capacity(32), // no growth in the callback for a normal call
        target: rate * TARGET_MS / 1000,
        refill: rate * TARGET_MS / 1000 + rate / 100,
        window: rate * WINDOW_MS / 1000,
        budget: rate * WINDOW_MS / 1000 * MAX_ADJUST_PER_MILLE / 1000,
        cut_above: rate * CUT_ABOVE_MS / 1000,
        pulled: 0,
        underrun,
        adjusted,
    };
    (input, speakers)
}

/// The receive side: one lock-free ring per peer into the speakers, plus what the UI and the
/// stats line read. Its locks are only ever taken by network-side tasks.
pub struct MixerInput {
    cmds: Arc<Mutex<Vec<Cmd>>>,
    pub(crate) peers: Mutex<HashMap<String, PeerIn>>,
    cap: usize,
    pushed: AtomicU64,
    underrun: Arc<AtomicU64>,
    adjusted: Arc<AtomicU64>,
}

pub(crate) struct PeerIn {
    ring: rtrb::Producer<i16>,
    gain: f32,
    /// Level of what we received (drives the speaking ring: lit = audible).
    meter: Meter,
    /// Queued for this peer as of the speakers' last pull.
    filled: Arc<AtomicUsize>,
}

impl MixerInput {
    fn peer<'a>(&self, peers: &'a mut HashMap<String, PeerIn>, id: &str) -> &'a mut PeerIn {
        if !peers.contains_key(id) {
            // twice the cap: a burst while the speakers reopen is trimmed on their side
            let (tx, rx) = rtrb::RingBuffer::new(2 * self.cap);
            let filled = Arc::new(AtomicUsize::new(0));
            self.cmds.lock().unwrap().push(Cmd::Add {
                id: id.to_string(),
                ring: rx,
                filled: filled.clone(),
            });
            peers.insert(
                id.to_string(),
                PeerIn { ring: tx, gain: 1.0, meter: Meter::default(), filled },
            );
        }
        peers.get_mut(id).expect("just added")
    }

    /// Queue received samples (mono i16).
    pub fn push(&self, peer: &str, samples: &[i16]) {
        self.pushed.fetch_add(samples.len() as u64, Relaxed);
        let mut peers = self.peers.lock().unwrap();
        let p = self.peer(&mut peers, peer);
        for s in samples {
            p.meter.add(*s as f32 / i16::MAX as f32 * p.gain);
        }
        // a full ring means the speakers aren't pulling (a reopen): keep the newest
        let newest = &samples[samples.len().saturating_sub(p.ring.slots())..];
        if let Ok(chunk) = p.ring.write_chunk_uninit(newest.len()) {
            chunk.fill_from_iter(newest.iter().copied());
        }
    }

    pub fn set_gain(&self, peer: &str, gain: f32) {
        let mut peers = self.peers.lock().unwrap();
        self.peer(&mut peers, peer).gain = gain;
        self.cmds.lock().unwrap().push(Cmd::Gain { id: peer.to_string(), gain });
    }

    pub fn remove(&self, peer: &str) {
        let removed = self.peers.lock().unwrap().remove(peer).is_some();
        if removed {
            self.cmds.lock().unwrap().push(Cmd::Remove { id: peer.to_string() });
        }
    }

    /// Forget every peer (a channel switch keeps the mixer, not the last room's people).
    pub fn clear(&self) {
        self.peers.lock().unwrap().clear();
        self.cmds.lock().unwrap().push(Cmd::Clear);
    }

    /// (peer, rms since the last call): what each person sounded like, after their volume.
    pub fn take_peer_levels(&self) -> Vec<(String, f32)> {
        self.peers
            .lock()
            .unwrap()
            .iter_mut()
            .map(|(id, p)| (id.clone(), p.meter.take().0))
            .collect()
    }

    pub fn take_underrun_samples(&self) -> u64 {
        self.underrun.swap(0, Relaxed)
    }

    pub fn take_adjusted_samples(&self) -> u64 {
        self.adjusted.swap(0, Relaxed)
    }

    pub fn pushed_total(&self) -> u64 {
        self.pushed.load(Relaxed)
    }

    /// Samples queued for the fullest peer, as of the speakers' last pull.
    pub fn max_buffered(&self) -> usize {
        let peers = self.peers.lock().unwrap();
        peers.values().map(|p| p.filled.load(Relaxed)).max().unwrap_or(0)
    }
}

/// The speaker side: owned by the output callback.
pub struct Mixer {
    cmds: Arc<Mutex<Vec<Cmd>>>,
    cap: usize,
    peers: HashMap<String, Peer>,
    target: usize,
    refill: usize,
    window: usize,
    budget: usize,
    cut_above: usize,
    pulled: usize,
    underrun: Arc<AtomicU64>,
    adjusted: Arc<AtomicU64>,
}

struct Peer {
    ring: rtrb::Consumer<i16>,
    /// What the speakers work on (trims, drift steps): sized once, never grown in the callback.
    queue: VecDeque<i16>,
    filled: Arc<AtomicUsize>,
    low: usize,
    gain: f32,
    adjust: i32,
    held: bool,
    rebuffering: bool,
}
```

`impl Mixer` keeps L7b's `mix_into` body with these changes: first line `self.intake();`; count into locals
`let (mut underrun, mut adjusted) = (0u64, 0u64);` instead of `self.underrun += 1` / `self.adjusted += …`, and at
the end

```rust
        for p in self.peers.values() {
            p.filled.store(p.queue.len() + p.ring.slots(), Relaxed);
        }
        self.underrun.fetch_add(underrun, Relaxed);
        self.adjusted.fetch_add(adjusted, Relaxed);
```

plus:

```rust
    /// Apply the network side's changes (only if they're not being written right now: never
    /// wait) and move what arrived from the rings into the working queues.
    fn intake(&mut self) {
        if let Ok(mut cmds) = self.cmds.try_lock() {
            for c in cmds.drain(..) {
                match c {
                    Cmd::Add { id, ring, filled } => {
                        let queue = VecDeque::with_capacity(3 * self.cap);
                        let p = Peer {
                            ring, queue, filled, low: usize::MAX, gain: 1.0,
                            adjust: 0, held: false, rebuffering: false,
                        };
                        self.peers.insert(id, p);
                    }
                    Cmd::Gain { id, gain } => {
                        if let Some(p) = self.peers.get_mut(&id) {
                            p.gain = gain;
                        }
                    }
                    Cmd::Remove { id } => {
                        self.peers.remove(&id);
                    }
                    Cmd::Clear => self.peers.clear(),
                }
            }
        }
        for p in self.peers.values_mut() {
            let n = p.ring.slots();
            if let Ok(chunk) = p.ring.read_chunk(n) {
                let (a, b) = chunk.as_slices();
                p.queue.extend(a.iter().chain(b).copied());
                chunk.commit_all();
            }
            if p.queue.len() > self.cap {
                let excess = p.queue.len() - self.cap;
                skip(&mut p.queue, excess); // oldest out, crossfaded
            }
        }
    }

    /// Samples queued for `peer` (tests and diagnostics; takes in what arrived first).
    pub fn buffered(&mut self, peer: &str) -> usize {
        self.intake();
        self.peers.get(peer).map_or(0, |p| p.queue.len())
    }
```

Delete the old `push`, `set_gain`, `remove`, `take_peer_levels`, `take_underrun_samples`, `pushed_total`,
`max_buffered` and `take_adjusted_samples` from `Mixer` (they live on `MixerInput` now).

`devices.rs` `Shared`: replace `pub mixer: Arc<Mutex<Mixer>>` with

```rust
    /// Speaker side of the mixer: only the output callback takes it (`try_lock`; the only other
    /// taker is a second output stream during a reopen).
    pub mixer: Mutex<Mixer>,
    /// Receive side: network tasks push here, lock-free towards the speakers.
    pub mix_in: Arc<MixerInput>,
    /// Last mute/deafen state the speaker callback saw (used when a toggle holds the lock).
    playout_on: std::sync::atomic::AtomicBool,
    /// Speaker callbacks that found the mixer busy and played silence (diagnostics).
    mix_busy: AtomicU64,
```

`Shared::new(cfg: &AudioConfig, controls: Arc<Mutex<Controls>>, peer_buffer_ms: u32)`:

```rust
        let (mix_in, mixer) = super::mixer::mixer(INTERNAL_RATE, peer_buffer_ms);
        let playout_on = controls.lock().unwrap().playout_on();
        Arc::new(Self {
            mixer: Mutex::new(mixer),
            mix_in: Arc::new(mix_in),
            playout_on: std::sync::atomic::AtomicBool::new(playout_on),
            mix_busy: AtomicU64::new(0),
            // … the other fields unchanged
        })
```

```rust
    pub fn take_mix_busy(&self) -> u64 {
        self.mix_busy.swap(0, Ordering::Relaxed)
    }
```

`on_output` (replaces `devices.rs:376-404`; the echo-reference part keeps its lock, see Why):

```rust
        let on = match self.controls.try_lock() {
            Ok(c) => {
                let on = c.playout_on();
                self.playout_on.store(on, Ordering::Relaxed);
                on
            }
            Err(_) => self.playout_on.load(Ordering::Relaxed), // a toggle in progress
        };
        let master = if on { 1.0 } else { 0.0 };
        let ch = channels.max(1);
        let frames = out.len() / ch;
        match self.mixer.try_lock() {
            Ok(mut mixer) => {
                if rate == INTERNAL_RATE || frames == 0 {
                    mixer.mix_into(out, channels, master);
                } else {
                    scratch.resize(frames, 0.0);
                    playout.fill(scratch, |buf| mixer.mix_into(buf, 1, master));
                    for (frame, v) in out.chunks_mut(ch).zip(scratch.iter()) {
                        frame.fill(*v);
                    }
                }
            }
            Err(_) => {
                out.fill(0.0);
                self.mix_busy.fetch_add(1, Ordering::Relaxed);
            }
        }
        // the meter is cosmetic: skip it rather than wait for the levels task
        let mut meter = self.spk_meter.try_lock().ok();
        let mut echo = self.echo_ref.lock().unwrap();
        if echo.rate != rate {
            echo.rate = rate;
            echo.samples.clear();
        }
        for v in downmix(out, channels) {
            if let Some(m) = meter.as_mut() {
                m.add(v);
            }
            echo.samples.push_back(to_i16(v));
        }
```

(the 200 ms cap lines after it stay). Update the `devices.rs` tests' `Shared::new(…, Arc::new(Mutex::new(Mixer::new(48_000, 200))))`
to `Shared::new(…, 200)`; same in `processor.rs` `shared_with`.

`mod.rs` (as it stands after L8): delete `let mixer = Arc::new(Mutex::new(Mixer::new(INTERNAL_RATE, PEER_BUFFER_MS)));`;
`let shared = Shared::new(&cfg, self.controls.clone(), PEER_BUFFER_MS);`. Then mechanically:
- stats task: `let underrun = shared.mix_in.take_underrun_samples(); let adjust = shared.mix_in.take_adjusted_samples();
  let mix_busy = shared.take_mix_busy();` and log `tracing::info!(%line, send_ms, adjust, mix_busy, %ours, "voice: path");`;
- room events task: capture `let mix_in = shared.mix_in.clone();` instead of `mixer`; `mixer.lock().unwrap().set_gain(…)`
  → `mix_in.set_gain(…)`; the receive task pushes with `m.push(&peer, &f.data)` where `m = mix_in.clone()`;
  `mixer.lock().unwrap().remove(&id)` → `mix_in.remove(&id)`;
- levels task: `shared.mixer.lock().unwrap().max_buffered()` → `shared.mix_in.max_buffered()`,
  `take_peer_levels()` likewise;
- `set_peer_volume`: `s.shared.mix_in.set_gain(&user_id, percent_to_gain(pct));`;
- `mixer_pushed`: `s.shared.mix_in.pushed_total()`;
- imports: `use mixer::percent_to_gain;` (drop `Mixer`).

`mictest.rs`: `Shared::new(cfg, Arc::new(Mutex::new(Controls::default())), 1000)`; `Loopback` holds
`mixer: Arc<MixerInput>` (`shared.mix_in.clone()`) and calls `self.mixer.push("self", &out)`; the levels task uses
`shared.mix_in.max_buffered()`.

- [ ] **Step 4: Run test to verify it passes**

`cargo test -p pulse-client --lib voice` → all pass (incl. L7b's drift tests on the split API and
`playback_never_waits_for_the_echo_canceller`). `just voice-it` → `receives_peer_audio_at_real_time_across_rejoins`
and `one_client_hears_another_at_real_time` pass (0.9-1.1× real time). Measurement: a CPU-stress call (a game or
all-core load) on Windows before/after: `voice: path underrun` ms and `mix_busy` (expect 0), compared with the
baseline's underrun count.

- [ ] **Step 5: Commit**

`git add client/src-tauri/Cargo.toml Cargo.lock client/src-tauri/src/voice/mixer.rs client/src-tauri/src/voice/devices.rs client/src-tauri/src/voice/mod.rs client/src-tauri/src/voice/mictest.rs client/src-tauri/src/voice/processor.rs`
`git commit -m "perf(voice): lock-free receive rings; the speaker callback never waits on network tasks"`

---

---

## Phase 4: Network and startup

### Task 12 (L1): Keep the HTTPS connection warm (HTTP/2, long idle, keepalive)

**Files:**
- Modify: `client/src-tauri/Cargo.toml:25` (reqwest features)
- Modify: `client/src-tauri/src/api.rs:115-131` (`Api::new`), `api.rs:136-179` (`parse`, `ok_empty`: log the negotiated version once)
- Modify: `deploy/Caddyfile:1-3` (global `servers { timeouts { idle } }`)
- Modify: `Cargo.lock` (h2 comes in with the feature)
- Test: `client/src-tauri/src/api.rs` (`mod tests`)

**Interfaces:**
- Consumes: `reqwest::ClientBuilder::{pool_idle_timeout, tcp_keepalive, http2_keep_alive_interval, http2_keep_alive_timeout, http2_keep_alive_while_idle}` (reqwest 0.12.28, `http2` feature).
- Produces: `pub const POOL_IDLE: Duration` in `api.rs`; log line `rest: connected version=HTTP/2.0` (once per process).

**Why:** journeys L4/V3/M1. reqwest drops an idle pooled connection after 90 s, so the first REST call after a lull
pays TCP + TLS again: +2 RTT, ~320 ms on the 160 ms Windows path. Join #2 in the Windows log paid exactly this
(+307 ms on `token + devices`). Expected: −320 ms on the voice token / first message after any lull > 90 s;
startup calls share one h2 connection instead of opening one each.

- [ ] **Step 1: Write the failing test**

Add to `api.rs` `mod tests` (needs `use std::sync::Arc; use std::sync::atomic::{AtomicUsize, Ordering};` at the
top of the test module):

```rust
    /// A bare HTTP/1.1 server that answers every request with 204 and never closes an idle
    /// connection itself (axum's test server drops idle ones after 30 s: hyper's header read
    /// timeout), counting the connections made to it.
    async fn keepalive_server() -> (String, Arc<AtomicUsize>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let l = tokio::net::TcpListener::bind("localhost:0").await.unwrap();
        let base = format!("http://localhost:{}", l.local_addr().unwrap().port());
        let conns = Arc::new(AtomicUsize::new(0));
        let count = conns.clone();
        tokio::spawn(async move {
            while let Ok((mut s, _)) = l.accept().await {
                count.fetch_add(1, Ordering::SeqCst);
                tokio::spawn(async move {
                    let (mut buf, mut chunk) = (Vec::new(), [0u8; 1024]);
                    loop {
                        let Ok(n) = s.read(&mut chunk).await else { return };
                        if n == 0 {
                            return;
                        }
                        buf.extend_from_slice(&chunk[..n]);
                        // the requests in these tests have no body: a blank line ends each one
                        while let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                            buf.drain(..end + 4);
                            if s.write_all(b"HTTP/1.1 204 No Content\r\n\r\n").await.is_err() {
                                return;
                            }
                        }
                    }
                });
            }
        });
        (base, conns)
    }

    #[tokio::test]
    async fn back_to_back_requests_share_one_connection() {
        let (base, conns) = keepalive_server().await;
        let api = Api::new(&base);
        api.logout("t").await.unwrap();
        api.logout("t").await.unwrap();
        assert_eq!(conns.load(Ordering::SeqCst), 1);
    }

    /// Field log 2026-10-13: a voice join 3.5 min after the last request paid ~300 ms more for
    /// its token (a new TCP + TLS handshake). The connection must survive a long lull.
    #[tokio::test]
    #[ignore = "idles 100 s; run: cargo test -p pulse-client --lib rest_connection -- --ignored"]
    async fn rest_connection_survives_a_long_idle() {
        let (base, conns) = keepalive_server().await;
        let api = Api::new(&base);
        api.logout("t").await.unwrap();
        tokio::time::sleep(Duration::from_secs(100)).await; // past reqwest's 90 s default
        api.logout("t").await.unwrap();
        assert_eq!(
            conns.load(Ordering::SeqCst),
            1,
            "the idle connection was dropped: the second call paid a new handshake"
        );
    }

    #[test]
    fn idle_connections_are_kept_longer_than_a_lull_but_shorter_than_caddy_keeps_them() {
        assert!(POOL_IDLE >= Duration::from_secs(30 * 60));
        assert!(POOL_IDLE < Duration::from_secs(60 * 60), "Caddy's idle timeout is 60 min");
    }
```

- [ ] **Step 2: Run test to verify it fails**

`cargo test -p pulse-client --lib api::tests::idle_connections -- --nocapture` → compile error
`cannot find value POOL_IDLE in this scope`.
`cargo test -p pulse-client --lib rest_connection -- --ignored` (after Step 3's const exists but before the builder
change, or on HEAD with the const stubbed) → `assertion left == right failed: the idle connection was dropped …
left: 2, right: 1`. `back_to_back_requests_share_one_connection` passes already (regression guard).

- [ ] **Step 3: Write minimal implementation**

`client/src-tauri/Cargo.toml:25`:

```toml
reqwest = { version = "0.12", default-features = false, features = ["json", "rustls-tls", "http2"] }
```

`api.rs`, above `pub struct Api`:

```rust
/// A fresh HTTPS connection costs two extra round trips (TCP + TLS: ~320 ms on a 160 ms path),
/// so keep the one we have. reqwest's default drops it after 90 s idle, which made most voice
/// joins and first messages after a lull pay for a new one. Caddy keeps idle connections for
/// 60 min (deploy/Caddyfile); we let go a little earlier so we never reuse one it just closed.
pub const POOL_IDLE: Duration = Duration::from_secs(55 * 60);
/// TCP keepalive and HTTP/2 pings keep NAT and firewall mappings alive under an idle connection.
const TCP_KEEPALIVE: Duration = Duration::from_secs(30);
const H2_PING_EVERY: Duration = Duration::from_secs(25);
const H2_PING_TIMEOUT: Duration = Duration::from_secs(10);

/// Logged once per run: the HTTP version the server agreed to (h2 through ALPN behind Caddy).
fn note_version(r: &reqwest::Response) {
    static ONCE: std::sync::Once = std::sync::Once::new();
    let version = r.version();
    ONCE.call_once(|| tracing::info!(?version, "rest: connected"));
}
```

`Api::new` (replaces `api.rs:121-125`):

```rust
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(15))
            .pool_idle_timeout(POOL_IDLE)
            .tcp_keepalive(TCP_KEEPALIVE)
            .http2_keep_alive_interval(H2_PING_EVERY)
            .http2_keep_alive_timeout(H2_PING_TIMEOUT)
            .http2_keep_alive_while_idle(true)
            .build()
            .expect("http client");
```

First line of the bodies of both `parse` and `ok_empty`:

```rust
        if let Ok(resp) = &r {
            note_version(resp);
        }
```

`deploy/Caddyfile:1-3` becomes:

```caddyfile
{
	email {$ACME_EMAIL}
	# Clients keep their HTTPS connection for 55 min between requests (client api.rs POOL_IDLE):
	# a new one costs two round trips. Caddy's default would close it after 5 min.
	servers {
		timeouts {
			idle 60m
		}
	}
}
```

Then `cargo build -p pulse-client` (the new feature adds `h2` to `Cargo.lock`; reqwest stays 0.12.28).

- [ ] **Step 4: Run test to verify it passes**

`cargo test -p pulse-client --lib api::tests` → all pass.
`cargo test -p pulse-client --lib rest_connection -- --ignored` → `1 passed` (~100 s).
Before/after measurement (Windows build, same path): join a voice channel 2+ min after the last REST call, read
`voice: mic published token_ms=` (baseline ~480 ms cold; expect ~160 ms) and check `rest: connected
version=HTTP/2.0` appears once. Caddy reload on the server needs the owner's OK (prod config).

- [ ] **Step 5: Commit**

`git add client/src-tauri/Cargo.toml Cargo.lock client/src-tauri/src/api.rs deploy/Caddyfile`
`git commit -m "perf(rest): keep the HTTPS connection warm (HTTP/2, 55 min idle, keepalive)"`

---

### Task 13 (L5): Startup: the gateway starts with the stored session at once, no `/me` first

**Files:**
- Modify: `client/src-tauri/src/gateway.rs:19-25` (add `Boot` next to `GatewayUpdate`)
- Modify: `client/src-tauri/src/commands.rs:17-38` (`Core` fields), `:48-50` (`activate`), `:113-147` (`forward`),
  `:182-205` (`restore_session`); add `start_stored`, `suspend`
- Modify: `client/src-tauri/src/lib.rs:111-118` (start the stored session at the end of `setup`)
- Test: `client/src-tauri/src/gateway.rs` (`mod tests`)

**Interfaces:**
- Consumes: `pulse_protocol::gateway::Ready { me: User, .. }` (verified: `protocol/src/gateway.rs:47-48`, Ready
  carries the user), `GatewayHandle::spawn_with`, close code 4001 → `ConnState::LoggedOut` (`gateway.rs:189`).
- Produces: `pub enum Boot { Waiting, Ready(User), LoggedOut, UpdateRequired, Unreachable }` with
  `pub fn observe(&mut self, u: &GatewayUpdate)` and `pub fn is_waiting(&self) -> bool`;
  `Core::start_stored(&self, app: &AppHandle) -> Option<String>`; `restore_session` keeps its signature
  (`Result<Option<User>, ApiError>`), so the UI is unchanged.

**Why:** journeys §1/L1. Today the gateway only starts after the webview booted, the JS asked `restore_session`, and
`/me` came back on a brand-new connection (~500-560 ms of serial round trips, `commands.rs:185-204`). Starting
from `setup` overlaps the gateway's connect with WebView2 + JS boot. Expected: Ready ~0.6-1.0 s sooner (baseline
`phase=ready` ~1.57 s after process start on Windows).

- [ ] **Step 1: Write the failing test**

Add to `gateway.rs` `mod tests`:

```rust
    async fn boot_outcome(base: String, token: &str) -> Boot {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let _gw = GatewayHandle::spawn_with(base, token.into(), tx, Duration::from_millis(50));
        let mut boot = Boot::Waiting;
        while boot.is_waiting() {
            boot.observe(&next(&mut rx).await);
        }
        boot
    }

    /// Startup no longer asks `/me`: Ready says who we are.
    #[tokio::test]
    async fn startup_learns_who_we_are_from_ready() {
        let app = testing::spawn().await;
        let (_, token) = testing::register(&app, "alex").await;
        match boot_outcome(format!("http://{}", app.addr), &token).await {
            Boot::Ready(me) => assert_eq!(me.username, "alex"),
            other => panic!("expected Ready, got {other:?}"),
        }
    }

    /// A stored token the server no longer accepts → the login screen, as with `/me`'s 401.
    #[tokio::test]
    async fn startup_with_a_dead_token_ends_logged_out() {
        let app = testing::spawn().await;
        let b = boot_outcome(format!("http://{}", app.addr), "garbage").await;
        assert!(matches!(b, Boot::LoggedOut), "{b:?}");
    }

    /// Nothing answers: say so (the UI shows "couldn't restore your last session").
    #[tokio::test]
    async fn startup_with_no_server_is_unreachable() {
        let b = boot_outcome("http://localhost:9".into(), "t").await;
        assert!(matches!(b, Boot::Unreachable), "{b:?}");
    }

    #[test]
    fn a_decided_boot_stays_decided() {
        let mut b = Boot::LoggedOut;
        b.observe(&GatewayUpdate::Connection(ConnState::Reconnecting));
        assert!(matches!(b, Boot::LoggedOut), "a later reconnect isn't startup");
    }
```

- [ ] **Step 2: Run test to verify it fails**

`cargo test -p pulse-client --lib gateway::tests::startup` → compile error `cannot find type Boot in this scope`.

- [ ] **Step 3: Write minimal implementation**

`gateway.rs`, after `enum GatewayUpdate` (line 25):

```rust
/// What the first gateway session says about a stored login. Startup no longer asks `/me`
/// first: Ready carries the user, a 4001 close means the token is dead.
#[derive(Debug, Clone)]
pub enum Boot {
    Waiting,
    Ready(pulse_protocol::rest::User),
    LoggedOut,
    UpdateRequired,
    /// The first attempt failed before Ready (no network, server down).
    Unreachable,
}

impl Boot {
    pub fn is_waiting(&self) -> bool {
        matches!(self, Boot::Waiting)
    }

    /// Fold in one update; once decided it stays (later reconnects aren't startup).
    pub fn observe(&mut self, u: &GatewayUpdate) {
        if !self.is_waiting() {
            return;
        }
        *self = match u {
            GatewayUpdate::Ready(r) => Boot::Ready(r.me.clone()),
            GatewayUpdate::Connection(ConnState::LoggedOut) => Boot::LoggedOut,
            GatewayUpdate::Connection(ConnState::UpdateRequired) => Boot::UpdateRequired,
            GatewayUpdate::Connection(ConnState::Reconnecting) => Boot::Unreachable,
            _ => return,
        };
    }
}
```

`commands.rs`: imports `use crate::gateway::{Boot, ConnState, GatewayHandle, GatewayUpdate};` and
`use tokio::sync::watch;`. New fields on `Core` (and in `Core::new`):

```rust
    /// The first gateway session's verdict on the stored login (restore_session waits on it).
    boot: watch::Sender<Boot>,
    /// The UI is listening for `pulse://…` events: the gateway may start before it is, so
    /// `forward` holds everything until then (the first Ready must not be emitted into the void).
    ui_attached: watch::Sender<bool>,
    /// setup and restore_session may both try to start the stored session: one at a time.
    starting: Mutex<()>,
```

```rust
            boot: watch::channel(Boot::Waiting).0,
            ui_attached: watch::channel(false).0,
            starting: Mutex::new(()),
```

First line of `activate`: `self.boot.send_replace(Boot::Waiting);`

New methods on `Core`:

```rust
    /// Start the stored session's live connection, once (setup and restore_session both ask; the
    /// first wins). The stored server, or `None` when nobody is signed in.
    pub(crate) fn start_stored(&self, app: &AppHandle) -> Option<String> {
        let _one = self.starting.lock().unwrap();
        if let Some((api, _)) = self.inner.lock().unwrap().clone() {
            return Some(api.base().to_string());
        }
        let (server, token) = self.store.load()?;
        self.activate(app, Api::new(&server), token, false);
        Some(server)
    }

    /// The stored session's server didn't answer at startup: stop trying, but keep it stored
    /// (the next launch tries again, as before).
    fn suspend(&self) {
        self.gateway.lock().unwrap().take();
        if let Some(t) = self.retry.lock().unwrap().take() {
            t.abort();
        }
        self.inner.lock().unwrap().take();
    }
```

`forward` (replaces `commands.rs:113-147`; the per-update handling is unchanged apart from matching on `&u`):

```rust
/// Gateway updates → Tauri events for the UI.
async fn forward(app: AppHandle, mut rx: mpsc::UnboundedReceiver<GatewayUpdate>) {
    let mut attached = app.state::<Core>().ui_attached.subscribe();
    let _ = attached.wait_for(|a| *a).await;
    while let Some(u) = rx.recv().await {
        match &u {
            GatewayUpdate::Ready(r) => {
                let _ = app.emit("pulse://ready", &**r);
            }
            GatewayUpdate::Event(e) => {
                let _ = app.emit("pulse://event", e);
            }
            GatewayUpdate::Connection(state) => {
                let state = *state;
                *app.state::<Core>().conn.lock().unwrap() = state;
                tracing::info!(?state, "gateway");
                if state == ConnState::LoggedOut {
                    app.state::<Core>().deactivate(&app);
                }
                if state == ConnState::Connected {
                    let core = app.state::<Core>();
                    if let Ok((api, token)) = core.current() {
                        let outbox = core.outbox.clone();
                        tauri::async_runtime::spawn(async move {
                            outbox.flush(&api, &token, true).await
                        });
                    }
                }
                let _ = app.emit("pulse://conn", state);
            }
        }
        // after the emit: restore_session returns only once the UI has the Ready
        app.state::<Core>().boot.send_modify(|b| b.observe(&u));
    }
}
```

`restore_session` (replaces `commands.rs:182-205`):

```rust
/// Longest startup waits for the first gateway session to decide (connect 10 s + Ready 10 s).
const BOOT_WAIT: std::time::Duration = std::time::Duration::from_secs(25);

/// Resume the stored session. `Ok(None)` means "show the login screen".
#[tauri::command]
pub async fn restore_session(
    app: AppHandle,
    core: State<'_, Core>,
) -> Result<Option<User>, ApiError> {
    crate::startup::mark("restore_begin");
    core.ui_attached.send_replace(true); // the UI listens before it calls this
    let Some(server) = core.start_stored(&app) else {
        return Ok(None);
    };
    let mut boot = core.boot.subscribe();
    let decided = tokio::time::timeout(BOOT_WAIT, boot.wait_for(|b| !b.is_waiting()))
        .await
        .ok()
        .and_then(|r| r.ok().map(|b| b.clone()));
    crate::startup::mark("restore_end");
    match decided {
        Some(Boot::Ready(user)) => Ok(Some(user)),
        Some(Boot::LoggedOut) => Ok(None), // forward() already cleared the stored session
        Some(Boot::UpdateRequired) => {
            // the update screen sits inside the signed-in shell: it needs the user (rare path)
            let (api, token) = core.current()?;
            api.me(&token).await.map(Some).map_err(|e| e.while_restoring(&server))
        }
        _ => {
            core.suspend();
            Err(ApiError::Network(crate::api::CONNECT_FAILED.into()).while_restoring(&server))
        }
    }
}
```

`lib.rs`, last statement of `setup` before `Ok(())` (after `VoiceManager` is managed, since `deactivate` uses it):

```rust
            // A stored login connects now, while the webview boots: Ready no longer waits for
            // the UI and a separate /me round trip. (Keychain reads block: not on the UI thread.)
            let h = app.handle().clone();
            tauri::async_runtime::spawn_blocking(move || {
                h.state::<commands::Core>().start_stored(&h);
            });
```

- [ ] **Step 4: Run test to verify it passes**

`cargo test -p pulse-client --lib gateway::tests` → all pass. `just check` (clippy, UI check) green.
Measurement: launch twice on Windows with a stored session; compare `startup phase=ready since_start_ms` (baseline
~1570 ms) and `phase=restore_end` against the step-0 report; expect Ready −600 to −1000 ms. Manual: revoke the
token (log out on another device) → the app opens on the login screen, as before.

- [ ] **Step 5: Commit**

`git add client/src-tauri/src/gateway.rs client/src-tauri/src/commands.rs client/src-tauri/src/lib.rs`
`git commit -m "perf(startup): connect the gateway from setup with the stored session; Ready replaces /me"`
**Failure mode covered here: startup on a machine whose network comes up late** (autostart at Windows login,
Wi-Fi still associating). With Steps 1-5, the first failed attempt before Ready means `Unreachable`: the login
screen with "couldn't restore your last session", and nothing retries. These steps give startup a grace period
(10 s) during which failed attempts keep retrying; only then is it `Unreachable` (`restore_session`'s 25 s
`BOOT_WAIT` still bounds the whole wait).

- [ ] **Step 6: Write the failing test**

`gateway.rs` `mod tests`. Change the `boot_outcome` helper to take the grace and start from `Boot::starting_with`:

```rust
    async fn boot_outcome(base: String, token: &str, grace: Duration) -> Boot {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let _gw = GatewayHandle::spawn_with(base, token.into(), tx, Duration::from_millis(50));
        let mut boot = Boot::starting_with(grace);
        while boot.is_waiting() {
            let u = next(&mut rx).await;
            boot.observe(&u, std::time::Instant::now());
        }
        boot
    }
```

(existing callers pass `Duration::from_secs(10)`; `startup_with_no_server_is_unreachable` passes
`Duration::from_millis(300)` so it stays fast; `a_decided_boot_stays_decided` calls
`b.observe(&…, std::time::Instant::now())`.) New:

```rust
    /// A TCP front that refuses (accepts and drops) for `down`, then forwards to `upstream`:
    /// the network coming up a moment after the app started.
    async fn late_network(upstream: std::net::SocketAddr, down: Duration) -> String {
        let l = tokio::net::TcpListener::bind("localhost:0").await.unwrap();
        let base = format!("http://localhost:{}", l.local_addr().unwrap().port());
        let up_at = tokio::time::Instant::now() + down;
        tokio::spawn(async move {
            while let Ok((mut s, _)) = l.accept().await {
                if tokio::time::Instant::now() < up_at {
                    drop(s); // no network yet
                    continue;
                }
                tokio::spawn(async move {
                    if let Ok(mut u) = tokio::net::TcpStream::connect(upstream).await {
                        let _ = tokio::io::copy_bidirectional(&mut s, &mut u).await;
                    }
                });
            }
        });
        base
    }

    #[tokio::test]
    async fn startup_survives_a_late_network() {
        let app = testing::spawn().await;
        let (_, token) = testing::register(&app, "alex").await;
        let base = late_network(app.addr, Duration::from_secs(1)).await;
        match boot_outcome(base, &token, Duration::from_secs(5)).await {
            Boot::Ready(me) => assert_eq!(me.username, "alex"),
            other => panic!("gave up on a network that came back after 1 s: {other:?}"),
        }
    }

    #[test]
    fn failures_within_the_grace_keep_waiting_then_give_up() {
        let t0 = std::time::Instant::now();
        let failed = GatewayUpdate::Connection(ConnState::Reconnecting);
        let mut b = Boot::starting_with(Duration::from_secs(10));
        b.observe(&failed, t0);
        b.observe(&failed, t0 + Duration::from_secs(9));
        assert!(b.is_waiting(), "{b:?}");
        b.observe(&failed, t0 + Duration::from_secs(10));
        assert!(matches!(b, Boot::Unreachable), "{b:?}");
    }
```

- [ ] **Step 7: Run test to verify it fails**

`cargo test -p pulse-client --lib gateway::tests` → compile errors `no function starting_with`, `observe takes 1
argument`. With only the signatures added (old rule: first `Reconnecting` → `Unreachable`):
`startup_survives_a_late_network` panics "gave up on a network that came back after 1 s: Unreachable" and
`failures_within_the_grace…` fails on `is_waiting`.

- [ ] **Step 8: Write minimal implementation**

`gateway.rs`, replacing Step 3's `Boot` (variants other than `Waiting` unchanged):

```rust
/// How long startup keeps retrying a stored session before saying the server is unreachable
/// (the network can come up a few seconds after an autostarted app).
pub const BOOT_GRACE: Duration = Duration::from_secs(10);

#[derive(Debug, Clone)]
pub enum Boot {
    /// No verdict yet. `failing_since`: when the first attempt before Ready failed.
    Waiting {
        failing_since: Option<std::time::Instant>,
        grace: Duration,
    },
    Ready(pulse_protocol::rest::User),
    LoggedOut,
    UpdateRequired,
    /// Attempts before Ready kept failing for the whole grace period.
    Unreachable,
}

impl Boot {
    pub fn starting() -> Self {
        Self::starting_with(BOOT_GRACE)
    }

    pub fn starting_with(grace: Duration) -> Self {
        Boot::Waiting { failing_since: None, grace }
    }

    pub fn is_waiting(&self) -> bool {
        matches!(self, Boot::Waiting { .. })
    }

    /// Fold in one update; once decided it stays (later reconnects aren't startup).
    pub fn observe(&mut self, u: &GatewayUpdate, now: std::time::Instant) {
        let Boot::Waiting { failing_since, grace } = self else {
            return;
        };
        *self = match u {
            GatewayUpdate::Ready(r) => Boot::Ready(r.me.clone()),
            GatewayUpdate::Connection(ConnState::LoggedOut) => Boot::LoggedOut,
            GatewayUpdate::Connection(ConnState::UpdateRequired) => Boot::UpdateRequired,
            GatewayUpdate::Connection(ConnState::Reconnecting) => {
                let since = *failing_since.get_or_insert(now);
                if now.saturating_duration_since(since) < *grace {
                    return; // keep retrying (the gateway's own backoff paces it)
                }
                Boot::Unreachable
            }
            _ => return,
        };
    }
}
```

`commands.rs`: `Boot::Waiting` → `Boot::starting()` in `Core::new` (`watch::channel(Boot::starting()).0`) and in
`activate` (`self.boot.send_replace(Boot::starting());`); in `forward`,
`app.state::<Core>().boot.send_modify(|b| b.observe(&u, std::time::Instant::now()));`. `restore_session` is
unchanged: a gateway still failing at `BOOT_WAIT` (25 s) falls into the `_` arm (`suspend` + "couldn't restore").
The gateway's backoff after a failed attempt is 0.8-1.2 s growing, so the 10 s grace sees several attempts.

- [ ] **Step 9: Run test to verify it passes**

`cargo test -p pulse-client --lib gateway::tests` → all pass (`startup_with_no_server_is_unreachable` in ~0.3 s,
`startup_survives_a_late_network` in ~1-2 s). Manual: start Pulse with Wi-Fi off, turn it on within 5 s → the
shell opens without the "couldn't restore" notice.

- [ ] **Step 10: Commit**

`git add client/src-tauri/src/gateway.rs client/src-tauri/src/commands.rs`
`git commit -m "fix(startup): keep retrying a stored session for 10 s before calling it unreachable"`


---

### Task 14 (R1): Dead gateway connections are noticed in seconds, not up to 75 s

**Why:** opt-journeys.md §5. On a silent blip (Wi-Fi drop, NAT rebind), the client only notices when no
frame arrives for 75 s (`gateway.rs:14-18` `HEARTBEAT` 30 s, `DEAD_AFTER` 75 s; `:194-205`). The UI's
`online`/`focus` kick (`client/ui/src/lib/store.svelte.ts:77-82`) only acts when the state already
reads `reconnecting`, but a half-open socket still reads `connected`, so it does nothing. After a
healthy session, the first retry still waits `backoff_delay(0)` = 0.8-1.2 s (`gateway.rs:121-128`,
`backoff.rs:8-11`). Changes:
- every heartbeat must be acked within max(3×RTT, 5 s), and heartbeats go every 15 s (the server's timeout is 60 s, `server/src/config.rs:34`);
- a probe (heartbeat plus a 3 s ack deadline) runs when the window gains focus, from Rust's `WindowEvent::Focused` so it doesn't depend on webview focus events, and when the webview says `online`;
- the first retry after a session that was up for at least 10 s is immediate, then the existing 1/2/4 s schedule.
A server that accepts and drops at once never counts as "healthy", so it can't cause a tight loop.

**Files:**
- Modify: `client/src-tauri/src/gateway.rs:1-18` (consts, imports), `:38-42` (`Cmd`), `:66-75` (`End`), `:77-158` (spawn functions), `:160-170` (handle methods), `:165-222` (`session`), tests `:238-` (new tests, `silent_server` helper)
- Modify: `client/src-tauri/src/backoff.rs` (add `reconnect_delay`, test)
- Modify: `client/src-tauri/src/commands.rs:117-122` (add `Core::probe_gateway`), `:219-225` (`gateway_reconnect_now` → `gateway_probe`)
- Modify: `client/src-tauri/src/lib.rs:117-126` (`.on_window_event`, handler list)
- Modify: `client/ui/src/lib/tauri.ts:24`, `client/ui/src/lib/store.svelte.ts:77-87`
- Test: `client/src-tauri/src/gateway.rs` (tests), `client/src-tauri/src/backoff.rs` (tests)

**Interfaces:**
- Consumes: `GatewayHandle`, `Core.gateway`, `backoff::backoff_delay`.
- Produces:
  - `pub struct gateway::Tuning { pub unit, pub heartbeat, pub dead_after, pub ack_min, pub probe_ack, pub healthy_after: Duration }` with `Default` = production values.
  - `pub fn GatewayHandle::spawn_tuning(base_url: String, token: String, tx: mpsc::UnboundedSender<GatewayUpdate>, t: Tuning) -> Self` (`spawn`, `spawn_with` and `spawn_tuned` keep their signatures and delegate to it).
  - `pub fn GatewayHandle::probe(&self)`: online → heartbeat and a `probe_ack` deadline; while waiting out a backoff → reconnect now.
  - `pub fn backoff::reconnect_delay(n: u32, after_healthy: bool, jitter: f64) -> Duration`
  - `pub(crate) fn commands::Core::probe_gateway(&self)`; Tauri command `gateway_probe` **replaces** `gateway_reconnect_now`; UI `api.probe()` replaces `api.reconnectNow()`.

- [ ] **Step 1: Write the failing tests**

`client/src-tauri/src/backoff.rs` tests:

```rust
    #[test]
    fn after_a_healthy_session_the_first_retry_is_immediate_then_the_usual_schedule() {
        let healthy: Vec<f64> = (0..4)
            .map(|n| reconnect_delay(n, true, 0.5).as_secs_f64())
            .collect();
        assert_eq!(healthy, vec![0.0, 1.0, 2.0, 4.0]);
        let cold: Vec<f64> = (0..3)
            .map(|n| reconnect_delay(n, false, 0.5).as_secs_f64())
            .collect();
        assert_eq!(cold, vec![1.0, 2.0, 4.0]);
    }
```

`client/src-tauri/src/gateway.rs` tests (add; the existing `silent_server_is_detected_as_dead` can switch
to the new helper, but doesn't have to):

```rust
    /// Serves Ready, then never says anything again (never acks, never closes): a half-open socket.
    async fn silent_server() -> std::net::SocketAddr {
        use futures::{SinkExt, StreamExt};
        use pulse_protocol::gateway::{Ready, ServerFrame};
        use pulse_protocol::rest::User;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            while let Ok((tcp, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let mut ws = tokio_tungstenite::accept_async(tcp).await.unwrap();
                    let _hello = ws.next().await;
                    let ready = ServerFrame::Ready(Ready {
                        me: User {
                            id: pulse_protocol::ids::UserId::new(),
                            username: "alex".into(),
                            avatar_hash: None,
                        },
                        servers: vec![],
                        channels: vec![],
                        members: vec![],
                        dm_members: vec![],
                        voice: vec![],
                        people: vec![],
                        read_states: vec![],
                        mutes: vec![],
                        hidden: vec![],
                        latest: vec![],
                    });
                    ws.send(tokio_tungstenite::tungstenite::Message::text(
                        serde_json::to_string(&ready).unwrap(),
                    ))
                    .await
                    .unwrap();
                    // keep reading (so writes succeed) but never answer
                    while ws.next().await.is_some() {}
                });
            }
        });
        addr
    }

    fn quick(heartbeat: Duration) -> Tuning {
        Tuning {
            unit: Duration::from_millis(50),
            heartbeat,
            dead_after: Duration::from_secs(60), // the old backstop: far beyond these tests
            ack_min: Duration::from_millis(200),
            probe_ack: Duration::from_millis(200),
            healthy_after: Duration::from_secs(10),
        }
    }

    async fn reconnecting_within(
        rx: &mut mpsc::UnboundedReceiver<GatewayUpdate>,
        limit: Duration,
    ) -> bool {
        tokio::time::timeout(limit, async {
            loop {
                match rx.recv().await {
                    Some(GatewayUpdate::Connection(ConnState::Reconnecting)) => return,
                    None => std::future::pending::<()>().await,
                    _ => {}
                }
            }
        })
        .await
        .is_ok()
    }

    /// A3: an unanswered heartbeat ends the session within its ack deadline, long before the
    /// no-frames backstop (60 s here).
    #[tokio::test]
    async fn unanswered_heartbeat_is_noticed_within_the_ack_deadline() {
        let addr = silent_server().await;
        let (tx, mut rx) = mpsc::unbounded_channel();
        let _gw = GatewayHandle::spawn_tuning(
            format!("http://{addr}"),
            "t".into(),
            tx,
            quick(Duration::from_millis(100)),
        );
        next_ready(&mut rx).await;
        let started = std::time::Instant::now();
        assert!(
            reconnecting_within(&mut rx, Duration::from_secs(2)).await,
            "dead connection not noticed"
        );
        assert!(started.elapsed() < Duration::from_secs(1), "{:?}", started.elapsed());
    }

    /// A3: window focus / network back probes a connection that still *looks* fine. A dead one is
    /// noticed within the probe's deadline, even with heartbeats a minute apart.
    #[tokio::test]
    async fn a_probe_notices_a_dead_connection_at_once() {
        let addr = silent_server().await;
        let (tx, mut rx) = mpsc::unbounded_channel();
        let gw = GatewayHandle::spawn_tuning(
            format!("http://{addr}"),
            "t".into(),
            tx,
            quick(Duration::from_secs(60)),
        );
        next_ready(&mut rx).await;
        gw.probe();
        assert!(reconnecting_within(&mut rx, Duration::from_secs(1)).await);
    }

    #[tokio::test]
    async fn a_probe_leaves_a_live_connection_alone() {
        let app = testing::spawn().await;
        let (_, token) = testing::register(&app, "alex").await;
        let (tx, mut rx) = mpsc::unbounded_channel();
        let gw = GatewayHandle::spawn_tuning(
            format!("http://{}", app.addr),
            token,
            tx,
            quick(Duration::from_secs(60)),
        );
        next_ready(&mut rx).await;
        gw.probe();
        assert!(
            !reconnecting_within(&mut rx, Duration::from_millis(800)).await,
            "a healthy connection was dropped by a probe"
        );
    }

    /// A3: a session that was fine reconnects straight away after a blip; the backoff (1 s here,
    /// as in production) only starts if that first try fails.
    #[tokio::test]
    async fn first_retry_after_a_healthy_session_is_immediate() {
        let app = testing::spawn().await;
        let (_, token) = testing::register(&app, "alex").await;
        let (tx, mut rx) = mpsc::unbounded_channel();
        let t = Tuning {
            healthy_after: Duration::ZERO, // any session counts as healthy
            ..Tuning::default()
        };
        let _gw = GatewayHandle::spawn_tuning(format!("http://{}", app.addr), token, tx, t);
        next_ready(&mut rx).await;
        app.hub.close_all(1012);
        loop {
            if let GatewayUpdate::Connection(ConnState::Reconnecting) = next(&mut rx).await {
                break;
            }
        }
        let lost = std::time::Instant::now();
        next_ready(&mut rx).await;
        assert!(
            lost.elapsed() < Duration::from_millis(500),
            "waited {:?} before reconnecting",
            lost.elapsed()
        );
    }

    #[test]
    fn ack_wait_is_three_round_trips_but_never_under_the_floor() {
        let mut l = Liveness::new(Duration::from_secs(5));
        assert_eq!(l.ack_wait(), Duration::from_secs(5));
        let t0 = Instant::now();
        l.sent(t0, l.ack_wait());
        l.heard(t0 + Duration::from_secs(2), true); // a 2 s round trip (a very slow link)
        assert_eq!(l.ack_wait(), Duration::from_secs(6));
        assert_eq!(l.due, None);
    }

    #[test]
    fn a_probe_shortens_the_wait_and_any_frame_answers_it() {
        let mut l = Liveness::new(Duration::from_secs(5));
        let t0 = Instant::now();
        l.sent(t0, Duration::from_secs(5));
        l.sent(t0 + Duration::from_secs(1), Duration::from_secs(3)); // probe 1 s later
        assert_eq!(l.due, Some(t0 + Duration::from_secs(4)));
        l.heard(t0 + Duration::from_millis(1500), false); // an event: alive
        assert_eq!(l.due, None);
        assert_eq!(l.rtt, None, "only an ack measures the round trip");
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p pulse-client --lib backoff gateway`
Expected: compile errors: `cannot find function reconnect_delay`, `cannot find type Tuning`,
`no function spawn_tuning`, `no method probe`, `cannot find type Liveness`.

- [ ] **Step 3: Write minimal implementation**

`client/src-tauri/src/backoff.rs` (after `backoff_delay`):

```rust
/// Reconnect schedule: `n` = attempts since the last session that reached Ready. After a healthy
/// session the first try is immediate (a blip, not an outage), then the usual 1 s, 2 s, 4 s …
pub fn reconnect_delay(n: u32, after_healthy: bool, jitter: f64) -> Duration {
    match (after_healthy, n) {
        (true, 0) => Duration::ZERO,
        (true, n) => backoff_delay(n - 1, jitter),
        (false, n) => backoff_delay(n, jitter),
    }
}
```

`client/src-tauri/src/gateway.rs`. Replace lines 12-18 with:

```rust
use tokio::time::Instant;

use crate::backoff::reconnect_delay;

const HEARTBEAT: Duration = Duration::from_secs(15);
const READY_TIMEOUT: Duration = Duration::from_secs(10);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Backstop: no frame at all for this long = dead, whatever the ack bookkeeping says.
const DEAD_AFTER: Duration = Duration::from_secs(75);
/// Least time a heartbeat gets for its ack; a slow link gets 3× its measured round trip.
const ACK_MIN: Duration = Duration::from_secs(5);
/// A probe (window focused, network back) must be answered within this.
const PROBE_ACK: Duration = Duration::from_secs(3);
/// Up at least this long after Ready = a healthy session: if it drops, retry at once.
const HEALTHY_AFTER: Duration = Duration::from_secs(10);

/// Timings (production by default; tests shrink them).
#[derive(Debug, Clone, Copy)]
pub struct Tuning {
    /// Scales the backoff schedule (1 s in production).
    pub unit: Duration,
    pub heartbeat: Duration,
    pub dead_after: Duration,
    pub ack_min: Duration,
    pub probe_ack: Duration,
    pub healthy_after: Duration,
}

impl Default for Tuning {
    fn default() -> Self {
        Self {
            unit: Duration::from_secs(1),
            heartbeat: HEARTBEAT,
            dead_after: DEAD_AFTER,
            ack_min: ACK_MIN,
            probe_ack: PROBE_ACK,
            healthy_after: HEALTHY_AFTER,
        }
    }
}

/// Whether the server still answers: every heartbeat (or probe) must be answered in time. Any
/// frame from the server counts as an answer; an ack also measures the round trip.
#[derive(Debug)]
struct Liveness {
    ack_min: Duration,
    /// When the oldest unanswered heartbeat went out.
    sent_at: Option<Instant>,
    /// Dead unless the server says something by then.
    due: Option<Instant>,
    rtt: Option<Duration>,
}

impl Liveness {
    fn new(ack_min: Duration) -> Self {
        Self {
            ack_min,
            sent_at: None,
            due: None,
            rtt: None,
        }
    }

    /// How long a regular heartbeat may wait for its ack.
    fn ack_wait(&self) -> Duration {
        self.rtt.map_or(self.ack_min, |r| (r * 3).max(self.ack_min))
    }

    /// A heartbeat went out at `now`; it must be answered within `wait` (a probe can only shorten
    /// an outstanding deadline, never extend it).
    fn sent(&mut self, now: Instant, wait: Duration) {
        if self.due.is_none() {
            self.sent_at = Some(now);
        }
        let due = now + wait;
        self.due = Some(self.due.map_or(due, |d| d.min(due)));
    }

    fn heard(&mut self, now: Instant, ack: bool) {
        self.due = None;
        if ack && let Some(at) = self.sent_at.take() {
            self.rtt = Some(now - at);
        }
    }
}

/// Sleeps until `at`, or forever when there's nothing to wait for.
async fn until(at: Option<Instant>) {
    match at {
        Some(t) => tokio::time::sleep_until(t).await,
        None => std::future::pending().await,
    }
}
```

`Cmd` (`:38-42`): add a `Probe` variant:

```rust
enum Cmd {
    Send(ClientFrame),
    ReconnectNow,
    /// Window focused / network back: prove the connection now (or stop waiting out a backoff).
    Probe,
    Stop,
}
```

`End` (`:66-75`): replace `Retry(bool)` with `Retry(Went)` and add:

```rust
/// How far a session that's to be retried got.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Went {
    /// Never reached Ready.
    Failed,
    /// Reached Ready but dropped within `healthy_after`.
    Brief,
    /// Was up for a while: the first retry is immediate.
    Healthy,
}
```

Spawn functions (`:77-158`): `spawn_with` becomes `Self::spawn_tuning(base_url, token, tx, Tuning { unit, ..Tuning::default() })`,
and `spawn_tuned` becomes `Self::spawn_tuning(base_url, token, tx, Tuning { unit, dead_after, ..Tuning::default() })`.
Add `spawn_tuning`, which takes the old `spawn_tuned` body with these changes:

```rust
    pub fn spawn_tuning(
        base_url: String,
        token: String,
        tx: mpsc::UnboundedSender<GatewayUpdate>,
        t: Tuning,
    ) -> Self {
        let (cmd, mut cmd_rx) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            let url = ws_url(&base_url);
            // attempts since the last session that reached Ready; whether that one was healthy
            let (mut n, mut after_healthy) = (0u32, false);
            let _ = tx.send(GatewayUpdate::Connection(ConnState::Connecting));
            loop {
                match session(&url, &token, &tx, &mut cmd_rx, &t).await {
                    End::Stop => return,
                    End::LoggedOut => {
                        let _ = tx.send(GatewayUpdate::Connection(ConnState::LoggedOut));
                        return;
                    }
                    End::UpdateRequired => {
                        let _ = tx.send(GatewayUpdate::Connection(ConnState::UpdateRequired));
                        return;
                    }
                    End::Retry(went) => {
                        if went != Went::Failed {
                            n = 0;
                            after_healthy = went == Went::Healthy;
                        }
                        let _ = tx.send(GatewayUpdate::Connection(ConnState::Reconnecting));
                        let delay = reconnect_delay(n, after_healthy, rand::random::<f64>())
                            .mul_f64(t.unit.as_secs_f64());
                        n = n.saturating_add(1);
                        // Wait out the backoff, unless told to reconnect now / probe, or stop.
                        let sleep = tokio::time::sleep(delay);
                        tokio::pin!(sleep);
                        loop {
                            tokio::select! {
                                _ = &mut sleep => break,
                                c = cmd_rx.recv() => match c {
                                    None | Some(Cmd::Stop) => return,
                                    Some(Cmd::ReconnectNow | Cmd::Probe) => break,
                                    Some(Cmd::Send(_)) => {} // dropped while offline
                                },
                            }
                        }
                    }
                }
            }
        });
        Self { cmd }
    }
```

Handle methods (next to `reconnect_now`):

```rust
    /// Check the connection now (window focused, network back): a dead one is noticed within
    /// seconds; while offline, retry at once.
    pub fn probe(&self) {
        let _ = self.cmd.send(Cmd::Probe);
    }
```

`session` (`:165-222`): change its last parameter `dead_after: Duration` to `t: &Tuning`. Every
pre-Ready `End::Retry(false)` becomes `End::Retry(Went::Failed)`. Replace everything from
`let mut beat = …` to the end of the loop with:

```rust
    let ready_at = Instant::now();
    // lost after Ready: a healthy session (retry at once) or a brief one (back off)
    let lost = || {
        End::Retry(if ready_at.elapsed() >= t.healthy_after {
            Went::Healthy
        } else {
            Went::Brief
        })
    };
    let heartbeat = serde_json::to_string(&ClientFrame::Heartbeat).expect("serialize");
    let mut beat = tokio::time::interval(t.heartbeat);
    beat.tick().await; // first tick is immediate
    // Only frames *from* the server prove the connection is alive (writes succeed into a dead socket).
    let mut deadline = Instant::now() + t.dead_after;
    let mut live = Liveness::new(t.ack_min);
    loop {
        tokio::select! {
            _ = tokio::time::sleep_until(deadline) => return lost(),
            _ = until(live.due) => {
                tracing::info!(rtt = ?live.rtt, "gateway: no answer in time; reconnecting");
                return lost();
            }
            _ = beat.tick() => {
                if ws.send(Ws::text(heartbeat.clone())).await.is_err() { return lost() }
                live.sent(Instant::now(), live.ack_wait());
            }
            frame = next_frame(&mut ws) => {
                let now = Instant::now();
                deadline = now + t.dead_after;
                live.heard(now, matches!(frame, Frame::Server(ServerFrame::HeartbeatAck)));
                match frame {
                    Frame::Server(ServerFrame::Event(e)) => { let _ = tx.send(GatewayUpdate::Event(Box::new(e))); }
                    Frame::Server(ServerFrame::Ready(r)) => { let _ = tx.send(GatewayUpdate::Ready(Box::new(r))); }
                    Frame::Server(ServerFrame::HeartbeatAck) => {}
                    Frame::Closed(Some(CloseCode::Library(4001))) => return End::LoggedOut,
                    Frame::Closed(Some(CloseCode::Library(4005))) => return End::UpdateRequired,
                    Frame::Closed(_) => return lost(),
                }
            }
            c = cmd_rx.recv() => match c {
                None | Some(Cmd::Stop) => { let _ = ws.close(None).await; return End::Stop }
                Some(Cmd::ReconnectNow) => return lost(),
                Some(Cmd::Probe) => {
                    if ws.send(Ws::text(heartbeat.clone())).await.is_err() { return lost() }
                    live.sent(Instant::now(), t.probe_ack);
                }
                Some(Cmd::Send(f)) => {
                    let f = serde_json::to_string(&f).expect("serialize");
                    if ws.send(Ws::text(f)).await.is_err() { return lost() }
                }
            },
        }
    }
```

Update the module doc (line 1) to: `//! Live connection to the server: Hello → Ready → events, heartbeats that must be answered in time, reconnect with backoff.`

`client/src-tauri/src/commands.rs`. After `send_frame` (`:117-122`):

```rust
    /// Check the live connection now (window focused, network back).
    pub(crate) fn probe_gateway(&self) {
        if let Some(g) = self.gateway.lock().unwrap().as_ref() {
            g.probe();
        }
    }
```

Replace `gateway_reconnect_now` (`:219-225`) with:

```rust
/// The UI calls this when the network comes back (`online`); window focus probes from Rust.
#[tauri::command]
pub fn gateway_probe(core: State<'_, Core>) {
    core.probe_gateway();
}
```

`client/src-tauri/src/lib.rs`. In the handler list, `commands::gateway_reconnect_now,` → `commands::gateway_probe,`.
Between `.setup(…)` and `.invoke_handler(…)`:

```rust
        .on_window_event(|window, event| {
            // Back at the app (or woken from sleep): a dead connection should show now, not at the
            // next heartbeat.
            if let tauri::WindowEvent::Focused(true) = event {
                use tauri::Manager;
                if let Some(core) = window.try_state::<commands::Core>() {
                    core.probe_gateway();
                }
            }
        })
```

`client/ui/src/lib/tauri.ts:24`: `reconnectNow: () => invoke<void>('gateway_reconnect_now'),` → `probe: () => invoke<void>('gateway_probe'),`

`client/ui/src/lib/store.svelte.ts:77-87`:

```ts
  // The network is back: check the connection now, whatever state it's in (a half-open socket
  // still reads "connected"). Window focus is probed from Rust.
  const probe = () => void api.probe()
  window.addEventListener('online', probe)
  return () => {
    offs.forEach((off) => off())
    window.removeEventListener('online', probe)
  }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p pulse-client --lib backoff gateway`
Expected: PASS (the new tests and the existing five gateway tests, including `silent_server_is_detected_as_dead` via `spawn_tuned`).
Run: `cargo test -p pulse-client --lib outbox` (spawns a real gateway). Expected: PASS.
Run: `cd client/ui && pnpm check && pnpm test`. Expected: PASS (no references to `reconnectNow` left: `grep -rn reconnectNow client/ui/src` prints nothing).
Run: `just check`. Expected: PASS.
Manual (Linux): `just dev-server` + `just dev-client`, sign in, then `sudo iptables -I OUTPUT -p tcp --dport 7890 -j DROP` (a silent drop). Expect `gateway state=Reconnecting` in the log within ~20 s. Click into the window and expect it within ~3 s. Then `sudo iptables -D OUTPUT -p tcp --dport 7890 -j DROP`.

- [ ] **Step 4b: Cover resume-from-sleep (a probe during a grown backoff)**

The failure mode is a laptop resuming with Wi-Fi not yet associated. The first tries fail, so the
backoff grows to 2-4 s. Wi-Fi then comes up, and the user would wait out the rest of the backoff.
The `online`/focus probe must cut that wait. R1's `Cmd::Probe` arm in the backoff loop already does
this, so no implementation change is needed. Refactor `silent_server` so it can serve on a given
listener, and add the test.

```rust
    /// Serves Ready on `listener`, then never says anything again.
    fn silent_server_on(listener: tokio::net::TcpListener) {
        use futures::{SinkExt, StreamExt};
        use pulse_protocol::gateway::{Ready, ServerFrame};
        use pulse_protocol::rest::User;
        tokio::spawn(async move {
            while let Ok((tcp, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let mut ws = tokio_tungstenite::accept_async(tcp).await.unwrap();
                    let _hello = ws.next().await;
                    let ready = ServerFrame::Ready(Ready {
                        me: User {
                            id: pulse_protocol::ids::UserId::new(),
                            username: "alex".into(),
                            avatar_hash: None,
                        },
                        servers: vec![],
                        channels: vec![],
                        members: vec![],
                        dm_members: vec![],
                        voice: vec![],
                        people: vec![],
                        read_states: vec![],
                        mutes: vec![],
                        hidden: vec![],
                        latest: vec![],
                    });
                    ws.send(tokio_tungstenite::tungstenite::Message::text(
                        serde_json::to_string(&ready).unwrap(),
                    ))
                    .await
                    .unwrap();
                    while ws.next().await.is_some() {}
                });
            }
        });
    }
```

The body of `silent_server()` (Step 1) becomes:

```rust
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        silent_server_on(listener);
        addr
```

The test:

```rust
    /// Resume from sleep: the network isn't back yet, so the first tries fail and the backoff
    /// grows (1 s, then 2 s: production schedule). When the network returns, the `online` / focus
    /// probe reconnects at once instead of waiting the backoff out.
    #[tokio::test]
    async fn a_probe_cuts_a_grown_backoff_short() {
        // a free port with nobody listening yet: connects are refused
        let addr = {
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap()
        };
        let (tx, mut rx) = mpsc::unbounded_channel();
        let gw = GatewayHandle::spawn_tuning(
            format!("http://{addr}"),
            "t".into(),
            tx,
            Tuning::default(),
        );
        // two failed tries: now waiting out the 2 s step
        let mut failures = 0;
        while failures < 2 {
            if let GatewayUpdate::Connection(ConnState::Reconnecting) = next(&mut rx).await {
                failures += 1;
            }
        }
        silent_server_on(tokio::net::TcpListener::bind(addr).await.unwrap()); // network back
        let t = std::time::Instant::now();
        gw.probe();
        next_ready(&mut rx).await;
        assert!(t.elapsed() < Duration::from_millis(500), "waited {:?}", t.elapsed());
    }
```

Run: `cargo test -p pulse-client --lib gateway::tests::a_probe_cuts_a_grown_backoff_short`
Expected: PASS in about 1.2 s (the first 1 s backoff, then the probe). Against pre-R1 code this test
doesn't compile: there is no `probe`, and `reconnect_now` was only sent while the UI showed `reconnecting`.

- [ ] **Step 5: Commit**

```bash
git add client/src-tauri/src/gateway.rs client/src-tauri/src/backoff.rs client/src-tauri/src/commands.rs client/src-tauri/src/lib.rs client/ui/src/lib/tauri.ts client/ui/src/lib/store.svelte.ts
git commit -m "fix(gateway): notice dead connections in seconds (ack deadline, probe on focus/online, immediate first retry)" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 15 (R6): Re-declare mute/deafen after every gateway Ready

**Why:** the server keeps each user's declared voice flags only in memory
(`server/src/voice/state.rs:31-40,95-104`, `declared`). The server section restores voice *presence* from
LiveKit after a restart, but the flags are gone, so everyone sees a muted user as unmuted. The client
sends `ClientFrame::VoiceState` only on join and on toggle (`commands.rs:374-377,393,403`). A toggle made
while offline is also dropped outright (`gateway.rs` backoff loop: `Some(Cmd::Send(_)) => {} // dropped while
offline`). Fix: the gateway sends a caller-supplied set of frames right after every Ready, and the app
supplies the current flags. This is sent always, not only while in voice. Declared flags are per user
whether or not they're in a room (`set_flags` stores them either way), and a frame the server already
has costs about 40 B. "Always" covers in voice, rejoining (R2) and about-to-join without coupling to
voice state.

**Files:**
- Modify: `client/src-tauri/src/gateway.rs` (an `AfterReady` type; `spawn_full`; `spawn_tuning` delegates; `session` sends the frames after Ready), tests
- Modify: `client/src-tauri/src/commands.rs:50-62` (`activate` passes the flags), `:344-349` (add `after_ready_frames` next to `flags`), tests
- Test: `client/src-tauri/src/gateway.rs` (tests), `client/src-tauri/src/commands.rs` (tests module; create `#[cfg(test)] mod tests` at the end if there is none)

**Interfaces:**
- Consumes: R1's `Tuning`, `Went`, `spawn_tuning`; `commands::flags(Controls)`; `VoiceManager::controls()`.
- Produces:
  - `pub type gateway::AfterReady = std::sync::Arc<dyn Fn() -> Vec<ClientFrame> + Send + Sync>`
  - `pub fn GatewayHandle::spawn_full(base_url: String, token: String, tx: mpsc::UnboundedSender<GatewayUpdate>, t: Tuning, after_ready: AfterReady) -> Self` (`spawn`, `spawn_with`, `spawn_tuned`, `spawn_tuning` are unchanged for callers and send nothing extra).
  - `fn commands::after_ready_frames(c: Controls) -> Vec<ClientFrame>`. B10 (gateway started in `setup`) must spawn through `activate` or pass the same closure.

- [ ] **Step 1: Write the failing tests**

`client/src-tauri/src/gateway.rs` tests:

```rust
    async fn wait_for(ok: impl Fn() -> bool) {
        tokio::time::timeout(Duration::from_secs(2), async {
            while !ok() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("condition never held");
    }

    /// R6: what the server keeps only in memory (our mute/deafen) is declared again after every
    /// Ready, so a server restart or a reconnect can't show us unmuted while we're muted.
    #[tokio::test]
    async fn after_ready_frames_are_sent_on_every_connect() {
        use pulse_protocol::gateway::VoiceFlags;
        let app = testing::spawn().await;
        let (me, token) = testing::register(&app, "alex").await;
        let muted = VoiceFlags {
            muted: true,
            deafened: false,
        };
        let after: AfterReady =
            std::sync::Arc::new(move || vec![ClientFrame::VoiceState { flags: muted }]);
        let (tx, mut rx) = mpsc::unbounded_channel();
        let _gw = GatewayHandle::spawn_full(
            format!("http://{}", app.addr),
            token,
            tx,
            quick(Duration::from_secs(60)),
            after,
        );
        next_ready(&mut rx).await;
        wait_for(|| app.voice.declared(me).muted).await;
        // a restart forgets what we declared; the reconnect must say it again
        app.voice.set_flags(me, VoiceFlags::default());
        app.hub.close_all(1012);
        next_ready(&mut rx).await;
        wait_for(|| app.voice.declared(me).muted).await;
    }
```

`client/src-tauri/src/commands.rs` tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// R6: after every Ready the client declares its current mute/deafen, whatever they are.
    #[test]
    fn after_ready_declares_the_current_flags() {
        let mut c = Controls::default();
        c.toggle_deafen(); // deafen implies mute
        let frames = after_ready_frames(c);
        assert_eq!(frames.len(), 1);
        let json = serde_json::to_value(&frames[0]).unwrap();
        let want = serde_json::to_value(ClientFrame::VoiceState {
            flags: pulse_protocol::gateway::VoiceFlags {
                muted: true,
                deafened: true,
            },
        })
        .unwrap();
        assert_eq!(json, want);
    }
}
```

(`commands.rs` has no tests module today, so this creates it. `Controls` has a private
`muted_before_deafen`, so the test builds it through `toggle_deafen`.)

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p pulse-client --lib gateway::tests::after_ready_frames_are_sent_on_every_connect commands::tests`
Expected: compile errors: `cannot find type AfterReady`, `no function spawn_full`, `cannot find function after_ready_frames`.

- [ ] **Step 3: Write minimal implementation**

`client/src-tauri/src/gateway.rs`, after `Tuning`:

```rust
/// Frames sent right after every Ready: state the server keeps only in memory (our mute/deafen),
/// which a server restart or a dropped offline send would otherwise lose.
pub type AfterReady = std::sync::Arc<dyn Fn() -> Vec<ClientFrame> + Send + Sync>;
```

Rename R1's `spawn_tuning` body to `spawn_full` with the extra parameter, and make `spawn_tuning` delegate:

```rust
    pub fn spawn_tuning(
        base_url: String,
        token: String,
        tx: mpsc::UnboundedSender<GatewayUpdate>,
        t: Tuning,
    ) -> Self {
        Self::spawn_full(base_url, token, tx, t, std::sync::Arc::new(Vec::<ClientFrame>::new))
    }

    pub fn spawn_full(
        base_url: String,
        token: String,
        tx: mpsc::UnboundedSender<GatewayUpdate>,
        t: Tuning,
        after_ready: AfterReady,
    ) -> Self {
        // (R1's spawn_tuning body; the only change is the session call:)
        //     match session(&url, &token, &tx, &mut cmd_rx, &t, &after_ready).await {
```

`session` gets a last parameter `after_ready: &AfterReady`. In the Ready arm, right after
`let _ = tx.send(GatewayUpdate::Connection(ConnState::Connected));`, and before the match returns:

```rust
            // what the server only keeps in memory (our mute/deafen): say it again on every Ready
            for f in after_ready() {
                let f = serde_json::to_string(&f).expect("serialize");
                if ws.send(Ws::text(f)).await.is_err() {
                    return End::Retry(Went::Brief);
                }
            }
```

`client/src-tauri/src/commands.rs`, after `fn flags` (`:344-349`):

```rust
/// Sent after every gateway Ready: the server forgets declared flags on restart, and a toggle
/// made while offline never reached it.
fn after_ready_frames(c: Controls) -> Vec<ClientFrame> {
    vec![ClientFrame::VoiceState { flags: flags(c) }]
}
```

In `Core::activate` (`:61`), replace `let handle = GatewayHandle::spawn(api.base().to_string(), token, tx);` with:

```rust
        let declare: crate::gateway::AfterReady = {
            let app = app.clone();
            std::sync::Arc::new(move || {
                // try_state: the gateway may start before voice is set up (B10)
                app.try_state::<VoiceManager>()
                    .map(|v| after_ready_frames(v.controls()))
                    .unwrap_or_default()
            })
        };
        let handle = GatewayHandle::spawn_full(
            api.base().to_string(),
            token,
            tx,
            crate::gateway::Tuning::default(),
            declare,
        );
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p pulse-client --lib gateway commands`
Expected: PASS (the new tests plus all of R1's gateway tests).
Run: `just check`. Expected: PASS.
Manual (Linux, `just dev-server` + two `just dev-client` profiles): A joins Lounge and mutes; restart the
dev server (Ctrl-C, `just dev-server`). Once A reconnects, B shows A muted (after the server section's
presence restore), without A touching mute.

- [ ] **Step 5: Commit**

```bash
git add client/src-tauri/src/gateway.rs client/src-tauri/src/commands.rs
git commit -m "fix(client): declare mute/deafen again after every gateway Ready (restarts and offline toggles lost them)" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 16 (L3): Prefetch the voice token on hover

**Files:**
- Create: `client/src-tauri/src/voice/tokens.rs`
- Modify: `client/src-tauri/src/voice/mod.rs:3-17` (module), `:290-318` (`VoiceManager` field + `prefetch`,
  `forget_tokens`), `:382` (`connect` takes a prefetched token first)
- Modify: `client/src-tauri/src/commands.rs` (new command `prefetch_voice`; `deactivate` forgets tokens)
- Modify: `client/src-tauri/src/lib.rs:120-161` (register `commands::prefetch_voice`)
- Modify: `client/ui/src/lib/voice.svelte.ts:69-93` (`voiceApi.prefetch`), `client/ui/src/components/ChannelList.svelte:84-87`
- Test: `client/src-tauri/src/voice/tokens.rs` (`mod tests`)

**Interfaces:**
- Consumes: `Api::voice_token(&self, token: &str, channel: ChannelId) -> Result<VoiceTokenResponse, ApiError>`
  (`VoiceTokenResponse: Clone`), server `TOKEN_TTL` = 10 min (`server/src/voice/routes.rs:20`).
- Produces: `pub struct TokenCache` with `prefetch(&self, &Api, session: &str, ChannelId, Instant)`,
  `async take(&self, session: &str, ChannelId, Instant) -> Option<VoiceTokenResponse>`, `clear(&self)`;
  `pub const FRESH_FOR: Duration` (120 s); `VoiceManager::prefetch(&self, &Api, &str, ChannelId)`,
  `VoiceManager::forget_tokens(&self)`; Tauri command `prefetch_voice { channelId }`; log field
  `token_prefetched` on the join line.

**Why:** journeys V2/S3. The token POST is 1 RTT warm (160 ms), 3 RTT cold (~480 ms) on the critical path of every
join and switch. Hover-to-click is usually 150-400 ms, which covers it. Expected: `token_ms` → ~0 when hovered
(−160 ms warm, −480 ms cold).

- [ ] **Step 1: Write the failing test**

`client/src-tauri/src/voice/tokens.rs` (test module first; the implementation goes above it in Step 3):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use pulse_server::testing;

    async fn setup() -> (testing::TestApp, Api, String, ChannelId, ChannelId) {
        let app = testing::spawn().await;
        let (_, token) = testing::register(&app, "alex").await;
        let s = testing::create_server(&app, &token, "Main").await;
        let text = testing::general(&app, &token, s.id).await.id;
        let api = Api::new(&format!("http://{}", app.addr));
        let lounge = api
            .channels(&token, s.id)
            .await
            .unwrap()
            .into_iter()
            .find(|c| c.kind == pulse_protocol::rest::ChannelKind::Voice)
            .unwrap()
            .id;
        (app, api, token, lounge, text)
    }

    #[tokio::test]
    async fn a_prefetched_token_is_used_once() {
        let (_app, api, token, lounge, _) = setup().await;
        let c = TokenCache::default();
        let now = Instant::now();
        c.prefetch(&api, &token, lounge, now);
        let t = c.take(&token, lounge, now).await.expect("prefetched");
        assert!(!t.token.is_empty());
        assert!(c.take(&token, lounge, now).await.is_none(), "a token is used once");
    }

    #[tokio::test]
    async fn a_click_during_the_prefetch_waits_for_it() {
        let (_app, api, token, lounge, _) = setup().await;
        let c = TokenCache::default();
        let now = Instant::now();
        c.prefetch(&api, &token, lounge, now);
        // no yield in between: the request is still in flight
        assert!(c.take(&token, lounge, now).await.is_some());
    }

    #[tokio::test]
    async fn stale_foreign_or_other_channel_prefetches_are_not_used() {
        let (_app, api, token, lounge, text) = setup().await;
        let c = TokenCache::default();
        let now = Instant::now();
        c.prefetch(&api, &token, lounge, now);
        assert!(c.take(&token, text, now).await.is_none(), "other channel");
        assert!(c.take("someone-else", lounge, now).await.is_none(), "other session");
        let late = now + FRESH_FOR + Duration::from_secs(1);
        assert!(c.take(&token, lounge, late).await.is_none(), "stale");
    }

    /// A prefetch that failed (here: a text channel has no voice) just means a normal fetch.
    #[tokio::test]
    async fn a_failed_prefetch_gives_nothing() {
        let (_app, api, token, _, text) = setup().await;
        let c = TokenCache::default();
        let now = Instant::now();
        c.prefetch(&api, &token, text, now);
        assert!(c.take(&token, text, now).await.is_none());
    }
}
```

Add `pub mod tokens;` to `voice/mod.rs:3-17` so it compiles into the crate.

- [ ] **Step 2: Run test to verify it fails**

`cargo test -p pulse-client --lib voice::tokens` → compile errors `cannot find type TokenCache`, `cannot find value FRESH_FOR`.

- [ ] **Step 3: Write minimal implementation**

Top of `voice/tokens.rs`:

```rust
//! Voice join tokens fetched ahead of the click (hovering a voice channel), each used once.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use futures::FutureExt;
use futures::future::{BoxFuture, Shared};
use pulse_protocol::ids::ChannelId;
use pulse_protocol::rest::VoiceTokenResponse;

use crate::api::Api;

/// The server's tokens last 10 min, but one only needs to be valid when the join starts. A
/// short shelf life keeps a token minted before a permission change from being used much later.
pub const FRESH_FOR: Duration = Duration::from_secs(120);

type Fetch = Shared<BoxFuture<'static, Option<VoiceTokenResponse>>>;

struct Entry {
    channel: ChannelId,
    /// Whose token it is: a different login must never reuse it.
    session: String,
    started: Instant,
    fetch: Fetch,
}

impl Entry {
    fn fits(&self, session: &str, channel: ChannelId, now: Instant) -> bool {
        self.channel == channel
            && self.session == session
            && now.saturating_duration_since(self.started) < FRESH_FOR
    }
}

/// One prefetched token (the channel last hovered): hover → click is the pattern it serves.
#[derive(Default)]
pub struct TokenCache {
    entry: Mutex<Option<Entry>>,
}

impl TokenCache {
    /// Start fetching `channel`'s token unless a fresh one is already here or on its way.
    pub fn prefetch(&self, api: &Api, session: &str, channel: ChannelId, now: Instant) {
        let mut slot = self.entry.lock().unwrap();
        if slot.as_ref().is_some_and(|e| e.fits(session, channel, now)) {
            return;
        }
        let (api, token) = (api.clone(), session.to_string());
        let fetch = async move { api.voice_token(&token, channel).await.ok() }
            .boxed()
            .shared();
        tokio::spawn(fetch.clone()); // shared futures are lazy: run it now, not at the click
        *slot = Some(Entry {
            channel,
            session: session.to_string(),
            started: now,
            fetch,
        });
    }

    /// The prefetched token for `channel`, waiting for it if still in flight. Used once; `None`
    /// if there's none (or it failed): fetch normally.
    pub async fn take(
        &self,
        session: &str,
        channel: ChannelId,
        now: Instant,
    ) -> Option<VoiceTokenResponse> {
        let fetch = {
            let mut slot = self.entry.lock().unwrap();
            if !slot.as_ref().is_some_and(|e| e.fits(session, channel, now)) {
                return None;
            }
            slot.take()?.fetch
        };
        fetch.await
    }

    pub fn clear(&self) {
        self.entry.lock().unwrap().take();
    }
}
```

`voice/mod.rs`: field `tokens: tokens::TokenCache,` on `VoiceManager` (init `tokens: Default::default(),`), and:

```rust
    /// Hovering a voice channel: fetch its join token now, so a click right after doesn't wait
    /// a round trip for it.
    pub fn prefetch(&self, api: &Api, token: &str, channel: ChannelId) {
        self.tokens.prefetch(api, token, channel, Instant::now());
    }

    /// Signed out: no token of this session may be used by the next.
    pub fn forget_tokens(&self) {
        self.tokens.clear();
    }
```

In `connect` replace `mod.rs:382` (`let vt = api.voice_token(token, channel).await?;`) with:

```rust
        let prefetched = self.tokens.take(token, channel, Instant::now()).await;
        let token_prefetched = prefetched.is_some();
        let vt = match prefetched {
            Some(vt) => vt,
            None => api.voice_token(token, channel).await?,
        };
```

and add `token_prefetched,` to the `tracing::info!(token_ms, …, "voice: mic published")` field list (`mod.rs:430-436`).

`commands.rs`, after `leave_voice`:

```rust
/// Hovering a voice channel (a click usually follows within 150-400 ms).
#[tauri::command]
pub async fn prefetch_voice(
    core: State<'_, Core>,
    voice: State<'_, VoiceManager>,
    channel_id: ChannelId,
) -> Result<(), ()> {
    if let Ok((api, token)) = core.current() {
        voice.prefetch(&api, &token, channel_id);
    }
    Ok(())
}
```

In `Core::deactivate` (before the `leave` spawn): `app.state::<VoiceManager>().forget_tokens();`.
`lib.rs` handler list: add `commands::prefetch_voice,` after `commands::leave_voice,`.

`client/ui/src/lib/voice.svelte.ts`, in `voiceApi`:

```ts
  /** Hovering a voice channel: the core fetches its join token ahead of the click. */
  prefetch: (channelId: string) => void invoke('prefetch_voice', { channelId }).catch(() => {}),
```

`client/ui/src/components/ChannelList.svelte`: `import { voiceApi } from '../lib/voice.svelte'` and on the voice row
button (line 84-87) add:

```svelte
      onmouseenter={() => { if (member && c.id !== voiceChannelId) voiceApi.prefetch(c.id) }}
      onfocus={() => { if (member && c.id !== voiceChannelId) voiceApi.prefetch(c.id) }}
```

- [ ] **Step 4: Run test to verify it passes**

`cargo test -p pulse-client --lib voice::tokens` → 4 passed. `cd client/ui && pnpm check && pnpm test` → green.
Measurement: hover then click a voice channel; `voice: mic published token_prefetched=true token_ms=` should read
~0-5 ms (baseline 160-480 ms).

- [ ] **Step 5: Commit**

`git add client/src-tauri/src/voice/tokens.rs client/src-tauri/src/voice/mod.rs client/src-tauri/src/commands.rs client/src-tauri/src/lib.rs client/ui/src/lib/voice.svelte.ts client/ui/src/components/ChannelList.svelte`
`git commit -m "perf(voice): prefetch the join token when a voice channel is hovered"`
**Failure mode covered here: hover prefetch, then the session changes.** Hover prefetches a token, the user logs
out and logs in as someone else within `FRESH_FOR`, and the click would join with the first account's token. The
cache is keyed by session already; these steps make sign-out empty it too and prove both.

- [ ] **Step 6: Write the failing test**

`voice/tokens.rs` `mod tests`:

```rust
    #[tokio::test]
    async fn a_cleared_cache_gives_nothing() {
        let (_app, api, token, lounge, _) = setup().await;
        let c = TokenCache::default();
        let now = Instant::now();
        c.prefetch(&api, &token, lounge, now);
        c.clear();
        assert!(c.take(&token, lounge, now).await.is_none(), "cleared on sign-out");
    }
```

`voice/mod.rs` `mod tests` (signing out goes through `Core::deactivate` → `forget_tokens`):

```rust
    #[tokio::test]
    async fn signing_out_forgets_prefetched_tokens() {
        let app = testing::spawn().await;
        let (_, token) = testing::register(&app, "alex").await;
        let s = testing::create_server(&app, &token, "Main").await;
        let api = Api::new(&format!("http://{}", app.addr));
        let lounge = api
            .channels(&token, s.id)
            .await
            .unwrap()
            .into_iter()
            .find(|c| c.kind == pulse_protocol::rest::ChannelKind::Voice)
            .unwrap();
        let vm = VoiceManager::new(Arc::new(|_| {}));
        vm.prefetch(&api, &token, lounge.id);
        vm.forget_tokens();
        assert!(vm.tokens.take(&token, lounge.id, Instant::now()).await.is_none());
    }
```

- [ ] **Step 7: Run test to verify it fails**

`cargo test -p pulse-client --lib voice::tokens::tests::a_cleared voice::tests::signing_out` → with Step 3's code
these pass if `clear`/`forget_tokens` are as written; they fail (`assertion failed: … is_none()`) if `clear` is a
no-op or `forget_tokens` is missing. Run them against a stubbed `clear() {}` once to see the failure, then restore.

- [ ] **Step 8: Write minimal implementation**

Already in Step 3: `TokenCache::clear` takes the entry, `VoiceManager::forget_tokens` calls it, and
`Core::deactivate` (`commands.rs`) calls `app.state::<VoiceManager>().forget_tokens();` **before** spawning the
leave. Check that last call is present: deactivate is the single path for logout and for a server-side 401/4001.

- [ ] **Step 9: Run test to verify it passes**

`cargo test -p pulse-client --lib voice::tokens voice::tests::signing_out` → all pass.

- [ ] **Step 10: Commit**

`git add client/src-tauri/src/voice/tokens.rs client/src-tauri/src/voice/mod.rs client/src-tauri/src/commands.rs`
`git commit -m "test(voice): prefetched tokens never outlive the session that fetched them"`


---

---

## Phase 5: Join and channel switch

### Task 17 (R2): Voice comes back by itself after LiveKit gives up

**Why:** opt-journeys.md §5 R3. LiveKit's SDK retries a broken call 10 times with backoff up to about
45 s (`reconnect_strategy.rs`). If all of those fail, `RoomEvent::Disconnected` arrives and Pulse tears
the session down for good (`voice/mod.rs:629-637`: `alive=false`, devices closed, UI `Disconnected`). The
user must click the channel again, and that includes after a LiveKit restart (`ServerShutdown`).
Change: for any reason other than ClientInitiated, DuplicateIdentity, ParticipantRemoved or
RoomDeleted, rejoin the same channel with Pulse's backoff (1, 2, 4, 8, 16, 30 s, ±20 %; 6 attempts, about
a minute more). Meanwhile the UI shows `Reconnecting` for that channel (`VoicePanel.svelte:24` already
renders it, and `transitionSounds` plays no sound for reconnecting → connected). Stop the moment the
user leaves or joins anything; that's an epoch check. Stop on 401. Report `Disconnected` when giving up.

**Files:**
- Create: `client/src-tauri/src/voice/rejoin.rs`
- Modify: `client/src-tauri/src/voice/mod.rs:1-17` (`pub mod rejoin;`), `:286-304` (`VoiceManager` fields), `:308-319` (`new`), `:329-367` (`join` → `join` + `join_inner`), `:371` (`connect`: capture epoch and sender), `:629-637` (Disconnected arm), `:763-790` (`leave`, `leave_locked`), `:833-852` (`toggled`), `:881-886` (`set_audio_config`), plus new methods, tests `:948-`
- Modify: `client/src-tauri/src/voice/mod.rs:96-150` (`Reopener`: `settle`, really close a reopen the session no longer wants)
- Modify: `client/src-tauri/src/voice/devices.rs:998-1072` (`AudioIo::close`)
- Modify: `client/src-tauri/src/lib.rs:111-117` (spawn the rejoin loop)
- Test: `client/src-tauri/src/voice/rejoin.rs` (tests), `client/src-tauri/src/voice/mod.rs` (tests), `client/src-tauri/src/voice/devices.rs` (tests)

**Interfaces:**
- Consumes: `backoff::backoff_delay`, `livekit::prelude::DisconnectReason` (`Copy`).
- Produces:
  - `voice::rejoin::{MAX_ATTEMPTS: u32 = 6, worth_rejoining(DisconnectReason) -> bool, delay(attempt: u32, jitter: f64) -> Duration}`
  - `pub async fn VoiceManager::run_rejoins(&self)`: runs for the app's life (spawned in `lib.rs`). Without it, behaviour is as before.
  - `pub fn VoiceManager::rejoins(&self) -> u32` (attempts so far, diagnostics) and `pub fn VoiceManager::rejoining(&self) -> Option<ChannelId>`.
  - Semantics other sections must keep: `join()` (user) and `leave()` bump `epoch` and set/clear `last_join`; `leave_locked(announce: bool) -> bool` (true if it tore down a session) first `settle()`s the reopener, and with `announce == false` closes the old devices and waits for them.
  - `pub fn devices::AudioIo::close(self)`: stop both sides and wait until they're really closed.

- [ ] **Step 1: Write the failing tests**

Create `client/src-tauri/src/voice/rejoin.rs` with the tests only:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejoins_unless_we_left_were_removed_or_the_room_is_gone() {
        use DisconnectReason::*;
        for r in [UnknownReason, ServerShutdown, SignalClose, JoinFailure, ConnectionTimeout, MediaFailure, StateMismatch, RoomClosed] {
            assert!(worth_rejoining(r), "{r:?}");
        }
        for r in [ClientInitiated, DuplicateIdentity, ParticipantRemoved, RoomDeleted] {
            assert!(!worth_rejoining(r), "{r:?}");
        }
    }

    #[test]
    fn about_a_minute_of_attempts_on_the_gateway_schedule() {
        let total: f64 = (0..MAX_ATTEMPTS).map(|a| delay(a, 0.5).as_secs_f64()).sum();
        assert_eq!(total, 1.0 + 2.0 + 4.0 + 8.0 + 16.0 + 30.0);
    }
}
```

In `client/src-tauri/src/voice/mod.rs` tests:

```rust
    async fn states_of(seen: &Mutex<Vec<VoiceEvent>>) -> Vec<(Option<ChannelId>, Connection)> {
        seen.lock()
            .unwrap()
            .iter()
            .filter_map(|e| match e {
                VoiceEvent::State { channel_id, connection, .. } => Some((*channel_id, *connection)),
                _ => None,
            })
            .collect()
    }

    /// A4: LiveKit gave up → voice rejoins by itself, says "reconnecting" for that channel the
    /// whole time (no "disconnected" flash), and gives up after MAX_ATTEMPTS. A text channel makes
    /// every attempt fail fast at the token step (no LiveKit needed).
    #[tokio::test]
    async fn a_dropped_call_is_rejoined_then_given_up_on() {
        let app = testing::spawn().await;
        let (_, token) = testing::register(&app, "alex").await;
        let s = testing::create_server(&app, &token, "Main").await;
        let general = testing::general(&app, &token, s.id).await;
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = seen.clone();
        let mut vm = VoiceManager::new(Arc::new(move |e| sink.lock().unwrap().push(e)));
        vm.rejoin_unit = Duration::from_millis(5);
        let vm = Arc::new(vm);
        let runner = {
            let vm = vm.clone();
            tokio::spawn(async move { vm.run_rejoins().await })
        };
        let api = Api::new(&format!("http://{}", app.addr));
        vm.simulate_drop(&api, &token, general.id).await;
        tokio::time::timeout(Duration::from_secs(5), async {
            while vm.rejoins() < rejoin::MAX_ATTEMPTS || vm.rejoining().is_some() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("rejoin never finished");
        let states = states_of(&seen).await;
        let (last, before) = states.split_last().unwrap();
        assert_eq!(*last, (None, Connection::Disconnected), "gave up: {states:?}");
        assert!(
            before.iter().all(|s| *s == (Some(general.id), Connection::Reconnecting)),
            "{states:?}"
        );
        assert_eq!(vm.rejoins(), rejoin::MAX_ATTEMPTS);
        runner.abort();
    }

    /// A4: the user's Leave wins over a rejoin in progress, at once and for good.
    #[tokio::test]
    async fn leaving_stops_a_rejoin() {
        let app = testing::spawn().await;
        let (_, token) = testing::register(&app, "alex").await;
        let s = testing::create_server(&app, &token, "Main").await;
        let general = testing::general(&app, &token, s.id).await;
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = seen.clone();
        let mut vm = VoiceManager::new(Arc::new(move |e| sink.lock().unwrap().push(e)));
        vm.rejoin_unit = Duration::from_millis(50);
        let vm = Arc::new(vm);
        let runner = {
            let vm = vm.clone();
            tokio::spawn(async move { vm.run_rejoins().await })
        };
        let api = Api::new(&format!("http://{}", app.addr));
        vm.simulate_drop(&api, &token, general.id).await;
        tokio::time::timeout(Duration::from_secs(3), async {
            while vm.rejoins() < 1 {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("no rejoin attempt");
        vm.leave().await;
        let n = vm.rejoins();
        tokio::time::sleep(Duration::from_millis(400)).await;
        assert_eq!(vm.rejoins(), n, "kept rejoining after Leave");
        assert_eq!(vm.rejoining(), None);
        assert_eq!(
            states_of(&seen).await.last(),
            Some(&(None, Connection::Disconnected))
        );
        runner.abort();
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p pulse-client --lib voice::rejoin voice::tests::a_dropped_call_is_rejoined_then_given_up_on voice::tests::leaving_stops_a_rejoin`
Expected: compile errors: `cannot find function worth_rejoining`, `no field rejoin_unit`, `no method run_rejoins / simulate_drop / rejoins / rejoining`.

- [ ] **Step 3: Write minimal implementation**

`client/src-tauri/src/voice/rejoin.rs` (above the tests):

```rust
//! Getting voice back after LiveKit gives up reconnecting (it tries for ~45 s on its own).

use std::time::Duration;

use livekit::prelude::DisconnectReason;

/// Rejoin attempts before giving up: with the delays below, about a minute more.
pub const MAX_ATTEMPTS: u32 = 6;

/// Whether losing the room this way is worth an automatic rejoin: not when we left, were removed,
/// were replaced by another login of ours, or the room is gone.
pub fn worth_rejoining(reason: DisconnectReason) -> bool {
    !matches!(
        reason,
        DisconnectReason::ClientInitiated
            | DisconnectReason::DuplicateIdentity
            | DisconnectReason::ParticipantRemoved
            | DisconnectReason::RoomDeleted
    )
}

/// Delay before rejoin attempt `attempt` (0-based): the gateway's schedule (1 s, 2 s, 4 s … ±20 %).
pub fn delay(attempt: u32, jitter: f64) -> Duration {
    crate::backoff::backoff_delay(attempt, jitter)
}
```

`client/src-tauri/src/voice/mod.rs`. Add `pub mod rejoin;` after `pub mod processor;`. Add above `pub struct VoiceManager`:

```rust
/// The user's last join, kept so a dropped call can be rejoined with the same settings.
#[derive(Clone)]
struct JoinArgs {
    api: Api,
    token: String,
    channel: ChannelId,
    cfg: AudioConfig,
    mode: AudioMode,
}

/// How a rejoin ended.
enum Rejoined {
    Yes,
    /// The user left or joined something else meanwhile: their action reported its own state.
    Superseded,
    GaveUp,
}
```

New fields at the end of `VoiceManager` (`:304`):

```rust
    /// Bumped by every user join/leave: a rejoin for an older one stands down.
    epoch: std::sync::atomic::AtomicU64,
    /// What the user last joined (the rejoin target); cleared on Leave.
    last_join: Mutex<Option<JoinArgs>>,
    /// Calls LiveKit dropped for good → `run_rejoins` (None until that loop runs: no rejoins).
    dropped_tx: Mutex<Option<mpsc::UnboundedSender<u64>>>,
    /// The channel being rejoined right now (the UI shows "reconnecting" for it).
    rejoining: Mutex<Option<ChannelId>>,
    rejoin_count: std::sync::atomic::AtomicU32,
    /// Scales the rejoin backoff (1 s in production; small in tests).
    rejoin_unit: Duration,
```

and in `new` (`:308-319`) add:

```rust
            epoch: Default::default(),
            last_join: Mutex::new(None),
            dropped_tx: Mutex::new(None),
            rejoining: Mutex::new(None),
            rejoin_count: Default::default(),
            rejoin_unit: Duration::from_secs(1),
```

Replace `join` (`:329-367`) with a user-facing `join` and the shared `join_inner`:

```rust
    /// Join `channel`, leaving any current room first (one voice connection per client).
    pub async fn join(
        &self,
        api: &Api,
        token: &str,
        channel: ChannelId,
        cfg: AudioConfig,
        mode: AudioMode,
    ) -> Result<(), VoiceError> {
        // the user's choice: supersedes any rejoin in progress
        self.epoch.fetch_add(1, Ordering::SeqCst);
        *self.last_join.lock().unwrap() = Some(JoinArgs {
            api: api.clone(),
            token: token.to_string(),
            channel,
            cfg: cfg.clone(),
            mode,
        });
        self.join_inner(api, token, channel, cfg, mode, None).await
    }

    /// `rejoin_of`: Some(epoch) for an automatic rejoin. It stands down if the user acted since,
    /// and it reports "reconnecting" instead of "connecting"/"disconnected".
    async fn join_inner(
        &self,
        api: &Api,
        token: &str,
        channel: ChannelId,
        cfg: AudioConfig,
        mode: AudioMode,
        rejoin_of: Option<u64>,
    ) -> Result<(), VoiceError> {
        let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();
        // a newer join supersedes an older pending one
        if let Some(old) = self.pending_join.lock().unwrap().replace(cancel_tx) {
            let _ = old.send(());
        }
        let _op = self.op.lock().await;
        if rejoin_of.is_some_and(|e| e != self.epoch.load(Ordering::SeqCst)) {
            return Err(VoiceError::Cancelled); // the user left or switched while we waited
        }
        self.leave_locked(rejoin_of.is_none()).await;
        tracing::info!(%channel, rejoin = rejoin_of.is_some(), "voice: joining");
        let trying = if rejoin_of.is_some() {
            Connection::Reconnecting
        } else {
            Connection::Connecting
        };
        self.emit_state(Some(channel), trying);
        let started = Instant::now();
        let outcome = tokio::select! {
            r = self.connect(api, token, channel, cfg, mode) => r,
            _ = cancel_rx => Err(VoiceError::Cancelled),
        };
        self.pending_join.lock().unwrap().take();
        let elapsed_ms = started.elapsed().as_millis() as u64;
        match &outcome {
            Ok(_) => tracing::info!(elapsed_ms, "voice: joined"),
            Err(e) => tracing::warn!(error = %e, elapsed_ms, "voice: join failed"),
        }
        match outcome {
            Ok(session) => {
                *self.live.lock().unwrap() = Some(session.alive.clone());
                *self.session.lock().await = Some(session);
                self.emit_state(Some(channel), Connection::Connected);
                Ok(())
            }
            Err(e) => {
                if rejoin_of.is_none() {
                    self.emit_state(None, Connection::Disconnected);
                }
                Err(e)
            }
        }
    }

    /// Brings voice back after LiveKit gives up: rejoins the same channel with backoff until it
    /// works, the user leaves or switches, or `rejoin::MAX_ATTEMPTS` fail. Runs for the app's life.
    pub async fn run_rejoins(&self) {
        let (tx, mut rx) = mpsc::unbounded_channel();
        *self.dropped_tx.lock().unwrap() = Some(tx);
        while let Some(epoch) = rx.recv().await {
            self.rejoin(epoch).await;
        }
    }

    async fn rejoin(&self, epoch: u64) {
        let Some(channel) = self
            .last_join
            .lock()
            .unwrap()
            .as_ref()
            .filter(|_| self.epoch.load(Ordering::SeqCst) == epoch)
            .map(|a| a.channel)
        else {
            return;
        };
        *self.rejoining.lock().unwrap() = Some(channel);
        let outcome = self.rejoin_attempts(epoch).await;
        *self.rejoining.lock().unwrap() = None;
        if let Rejoined::GaveUp = outcome {
            tracing::warn!(%channel, "voice: couldn't rejoin; giving up");
            self.emit_state(None, Connection::Disconnected);
        }
    }

    async fn rejoin_attempts(&self, epoch: u64) -> Rejoined {
        for attempt in 0..rejoin::MAX_ATTEMPTS {
            let wait = rejoin::delay(attempt, rand::random::<f64>())
                .mul_f64(self.rejoin_unit.as_secs_f64());
            tokio::time::sleep(wait).await;
            let args = self.last_join.lock().unwrap().clone();
            let Some(a) = args.filter(|_| self.epoch.load(Ordering::SeqCst) == epoch) else {
                return Rejoined::Superseded;
            };
            self.rejoin_count.fetch_add(1, Ordering::Relaxed);
            tracing::info!(attempt, channel = %a.channel, "voice: rejoining (connection lost)");
            match self
                .join_inner(&a.api, &a.token, a.channel, a.cfg.clone(), a.mode, Some(epoch))
                .await
            {
                Ok(()) => return Rejoined::Yes,
                Err(VoiceError::Cancelled) => return Rejoined::Superseded,
                Err(VoiceError::Api(ApiError::Unauthorized)) => return Rejoined::GaveUp,
                Err(e) => tracing::warn!(attempt, error = %e, "voice: rejoin failed"),
            }
        }
        Rejoined::GaveUp
    }

    /// Rejoin attempts so far (diagnostics).
    pub fn rejoins(&self) -> u32 {
        self.rejoin_count.load(Ordering::Relaxed)
    }

    /// The channel being rejoined right now, if any.
    pub fn rejoining(&self) -> Option<ChannelId> {
        *self.rejoining.lock().unwrap()
    }

    /// Tests: act as if LiveKit just gave up on a call in `channel` the user had joined.
    #[cfg(test)]
    async fn simulate_drop(&self, api: &Api, token: &str, channel: ChannelId) {
        let epoch = self.epoch.fetch_add(1, Ordering::SeqCst) + 1;
        *self.last_join.lock().unwrap() = Some(JoinArgs {
            api: api.clone(),
            token: token.into(),
            channel,
            cfg: AudioConfig::default(),
            mode: AudioMode::Null(48_000),
        });
        let tx = loop {
            if let Some(tx) = self.dropped_tx.lock().unwrap().clone() {
                break tx;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        };
        self.emit_state(Some(channel), Connection::Reconnecting);
        tx.send(epoch).unwrap();
    }
```

In `connect` (`:371`), before the `// room events` block (`:549`), capture what the events task needs:

```rust
        // a drop LiveKit gives up on is handed to the rejoin loop (if it runs) under this join's epoch
        let (epoch, dropped) = (
            self.epoch.load(Ordering::SeqCst),
            self.dropped_tx.lock().unwrap().clone(),
        );
```

and replace the `RoomEvent::Disconnected` arm (`:629-637`):

```rust
                        RoomEvent::Disconnected { reason } => {
                            tracing::warn!(?reason, "voice disconnected by LiveKit");
                            // Gone: stop the mic/speakers and mark the session dead.
                            let was_alive = alive.swap(false, Ordering::SeqCst);
                            io.lock().unwrap().take();
                            rx.lock().unwrap().clear();
                            let handed_over = was_alive
                                && rejoin::worth_rejoining(reason)
                                && dropped.as_ref().is_some_and(|d| d.send(epoch).is_ok());
                            if handed_over {
                                events(state(Connection::Reconnecting, Some(channel)));
                            } else {
                                events(state(Connection::Disconnected, None));
                            }
                            break;
                        }
```

`leave` / `leave_locked` (`:763-790`):

```rust
    pub async fn leave(&self) {
        // the user's choice: a rejoin in progress stands down
        self.epoch.fetch_add(1, Ordering::SeqCst);
        let was_joined = self.last_join.lock().unwrap().take().is_some();
        // Cancel a join that's still connecting first, so we never queue behind it.
        if let Some(cancel) = self.pending_join.lock().unwrap().take() {
            tracing::info!("voice: leave cancels an in-flight join");
            let _ = cancel.send(());
        }
        let _op = self.op.lock().await;
        if !self.leave_locked(true).await && was_joined {
            // nothing to tear down (a rejoin was between attempts): still say we're out
            self.emit_state(None, Connection::Disconnected);
        }
    }

    /// Tear down the session, if any (true if there was one). `announce`: tell the UI
    /// "disconnected" (a rejoin replaces the dead session without saying so).
    async fn leave_locked(&self, announce: bool) -> bool {
        let Some(s) = self.session.lock().await.take() else {
            return false;
        };
        tracing::info!("voice: leaving");
        // first: a reopen in flight must not open devices for a session that's gone
        s.alive.store(false, Ordering::SeqCst);
        if tokio::time::timeout(CLOSE_TIMEOUT, s.room.close())
            .await
            .is_err()
        {
            tracing::warn!("voice: room close timed out; dropping it");
        }
        s.rx.lock().unwrap().clear();
        s.io.lock().unwrap().take();
        drop(s);
        if announce {
            self.emit_state(None, Connection::Disconnected);
        }
        true
    }
```

`toggled` (`:833-852`): replace the `channel`/`connection` computation with:

```rust
        // The audio callbacks share `controls`, so this already took effect; just tell the UI.
        let (channel_id, connection) = match (self.current_channel().await, self.rejoining()) {
            (Some(c), _) => (Some(c), Connection::Connected),
            (None, Some(c)) => (Some(c), Connection::Reconnecting),
            (None, None) => (None, Connection::Disconnected),
        };
        (self.events)(VoiceEvent::State {
            channel_id,
            connection,
            controls: c,
        });
```

`set_audio_config` (`:881`): as the first statement, so a rejoin uses the newest settings:

```rust
        if let Some(a) = self.last_join.lock().unwrap().as_mut() {
            a.cfg = cfg.clone();
        }
```

`client/src-tauri/src/lib.rs`, right after `app.manage(voice::VoiceManager::new(…));` (`:111-116`):

```rust
            // voice comes back by itself after LiveKit gives up reconnecting
            let h = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                h.state::<voice::VoiceManager>().run_rejoins().await
            });
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p pulse-client --lib voice::rejoin voice::tests`
Expected: PASS (the new tests plus `failed_join_reports_disconnected`, `leave_cancels_a_stuck_join` and the serialisation tests).
Run: `just dev-livekit && just voice-it`. Expected: PASS (the join/leave paths are unchanged for user actions).
Run: `just check`. Expected: PASS.
Manual check on a real LiveKit (Linux): join Lounge in `just dev-client`, run `docker compose pause livekit`
for 70 s, then `docker compose unpause livekit`. Expect in the log: `voice: reconnecting` → (≈45-60 s)
`voice disconnected by LiveKit reason=…` → `voice: rejoining (connection lost) attempt=…` → `voice: joined`,
with the panel showing "Reconnecting…" the whole time and no join/leave sound. Then click Leave during a
rejoin and expect it to stop (no further `voice: rejoining` lines).

- [ ] **Step 4b: A rejoin must not race a device reopen in flight (Bluetooth headsets)**

The failure mode is LiveKit giving up while `Reopener::reopen` (stall recovery, device following or a
settings change) has the old `AudioIo` out on a blocking thread. When that reopen finishes,
`alive` is false, so it **drops** the new streams (`mod.rs:139-149`). `Parked`'s `Drop` only *asks* the
audio thread to stop (`devices.rs:557-561`). `leave_locked` also only drops `s.io`. The rejoin's
`start_audio` then opens the same devices at once. A Bluetooth headset can't be open twice, so every
attempt fails with a device error. Fix it in three places:
- the teardown waits (bounded) for any reopen in flight;
- a reopen that finds the session gone really closes what it opened;
- a rejoin's teardown closes the old devices and waits until they are gone.

The user's own Leave keeps the fast drop.

Failing tests first. In `client/src-tauri/src/voice/devices.rs` tests:

```rust
    /// A rejoin reopens the same devices right after tearing down: `close` must not return until
    /// the old streams are really gone (a Bluetooth headset can't be open twice).
    #[test]
    fn close_waits_until_the_streams_are_really_gone() {
        use std::sync::atomic::AtomicBool;
        struct SlowStream(Arc<AtomicBool>);
        impl Drop for SlowStream {
            fn drop(&mut self) {
                std::thread::sleep(Duration::from_millis(200)); // a driver taking its time
                self.0.store(true, Ordering::SeqCst);
            }
        }
        let gone = Arc::new(AtomicBool::new(false));
        let g = gone.clone();
        let half = Parked::spawn("test-half", Duration::from_secs(1), move || {
            Ok((
                SlowStream(g),
                HalfInfo {
                    rate: 48_000,
                    name: "test".into(),
                    generation: 0,
                },
            ))
        })
        .unwrap();
        let (raw_tx, _raw_rx) = std::sync::mpsc::channel();
        let shared = Shared::new(
            &AudioConfig::default(),
            Default::default(),
            Arc::new(Mutex::new(crate::voice::mixer::Mixer::new(48_000, 200))),
        );
        let io = AudioIo {
            input: Some(half),
            output: None,
            null: None,
            shared,
            raw_tx,
        };
        io.close();
        assert!(gone.load(Ordering::SeqCst), "returned before the stream closed");
    }
```

In `client/src-tauri/src/voice/mod.rs` tests:

```rust
    fn idle_reopener() -> Arc<Reopener> {
        let cfg = AudioConfig::default();
        let shared = Shared::new(
            &cfg,
            Default::default(),
            Arc::new(Mutex::new(Mixer::new(INTERNAL_RATE, PEER_BUFFER_MS))),
        );
        let (raw_tx, _raw_rx) = std::sync::mpsc::channel();
        Arc::new(Reopener {
            cfg: Arc::new(Mutex::new(cfg)),
            io: Arc::new(Mutex::new(None)),
            shared,
            raw_tx,
            alive: Arc::new(AtomicBool::new(false)),
            one_at_a_time: tokio::sync::Mutex::new(()),
            failed: Default::default(),
            count: Default::default(),
        })
    }

    /// A rejoin's teardown waits for a device reopen that's still running, so it doesn't open
    /// the same (Bluetooth) device while the reopen still holds it.
    #[tokio::test]
    async fn teardown_waits_for_a_device_reopen_in_flight() {
        let r = idle_reopener();
        let held = {
            let r = r.clone();
            tokio::spawn(async move {
                let _turn = r.one_at_a_time.lock().await; // a reopen in flight
                tokio::time::sleep(Duration::from_millis(300)).await;
            })
        };
        tokio::time::sleep(Duration::from_millis(20)).await;
        let t = Instant::now();
        r.settle().await;
        assert!(t.elapsed() >= Duration::from_millis(250), "{:?}", t.elapsed());
        held.await.unwrap();
    }
```

Run: `cargo test -p pulse-client --lib voice::devices::tests::close_waits_until_the_streams_are_really_gone voice::tests::teardown_waits_for_a_device_reopen_in_flight`
Expected: compile errors: `no method named close found for struct AudioIo`, `no method named settle found for Arc<Reopener>`.

Implementation. In `client/src-tauri/src/voice/devices.rs` `impl AudioIo` (after `empty`):

```rust
    /// Stop both sides and wait (up to 1 s each) until they're really closed. Dropping only asks;
    /// use this when the same devices are about to be opened again.
    pub fn close(mut self) {
        for half in [self.input.take(), self.output.take()].into_iter().flatten() {
            half.close();
        }
    }
```

In `client/src-tauri/src/voice/mod.rs`, near the other constants:

```rust
/// How long a teardown waits for a device reopen in flight: `open_half`'s own 5 s limit plus a close.
const SETTLE_WAIT: Duration = Duration::from_secs(6);
```

In `impl Reopener`, add:

```rust
    /// Wait (bounded) until no reopen is in flight. Call after clearing `alive`: a reopen that
    /// finishes now closes what it opened instead of keeping it.
    async fn settle(&self) {
        if tokio::time::timeout(SETTLE_WAIT, self.one_at_a_time.lock())
            .await
            .is_err()
        {
            tracing::warn!("voice: a device reopen is still running; going ahead");
        }
    }
```

In `Reopener::reopen`, replace the `if let Some(io) = io && self.alive.load(…) { … }` block (`:139-149`) with:

```rust
        match io {
            Some(io) if self.alive.load(Ordering::SeqCst) => {
                tracing::info!(?sides, "voice: devices reopened");
                self.shared.watchdog.rearm(Instant::now());
                self.failed
                    .lock()
                    .unwrap()
                    .record(sides, &targets, &io.opened(), Instant::now());
                *self.io.lock().unwrap() = Some(io);
            }
            // the session ended meanwhile: really close (a rejoin may open the same device next)
            Some(io) => {
                let _ = tokio::task::spawn_blocking(move || io.close()).await;
            }
            None => {}
        }
```

In `leave_locked` (as rewritten in Step 3), right after `s.alive.store(false, Ordering::SeqCst);`:

```rust
        s.reopener.settle().await;
```

and replace `s.io.lock().unwrap().take();` with:

```rust
        let io = s.io.lock().unwrap().take();
        if let Some(io) = io {
            if announce {
                drop(io); // the user's Leave: don't wait on the devices
            } else {
                // a rejoin opens the same devices next: wait until these are really closed
                let _ = tokio::task::spawn_blocking(move || io.close()).await;
            }
        }
```

Run: `cargo test -p pulse-client --lib voice::devices voice::tests`
Expected: PASS (both new tests plus all of R2's).
Run: `just dev-livekit && just voice-it`. Expected: PASS (`reopening_one_side_leaves_the_other_alone` and `rapid_join_leave_never_hangs` exercise the changed paths).

- [ ] **Step 5: Commit**

```bash
git add client/src-tauri/src/voice/rejoin.rs client/src-tauri/src/voice/mod.rs client/src-tauri/src/voice/devices.rs client/src-tauri/src/lib.rs
git commit -m "feat(voice): rejoin by itself after LiveKit gives up reconnecting" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 18 (L2a): Voice join: open devices while the room connects

**Files:**
- Modify: `client/src-tauri/src/voice/mod.rs:371-405` (`connect`: device open ∥ `Room::connect`), new
  `side_by_side` helper
- Modify: `client/src-tauri/src/voice/devices.rs:999-1011` (`AudioIo::start`: input ∥ output)
- Test: `client/src-tauri/src/voice/mod.rs` (`mod tests`)
- cpal: `AudioIo::start` calls `open_half` (cpal `find_device`, `supported_*_configs`, `build_*_stream`). Only the
  orchestration changes here; the cpal-upgrade section edits `open_half` itself. Merge order doesn't matter, but
  re-run `real_devices_are_not_reopened_without_a_reason` (voice_it) after both.

**Interfaces:**
- Consumes: `start_audio(&AudioConfig, Arc<Shared>, RawTx) -> Result<AudioIo, VoiceError>`,
  `Room::connect(&str, &str, RoomOptions)`, `Room::close()`.
- Produces: `async fn side_by_side<D, R, F>(devices: impl Future<Output = Result<D, VoiceError>>, room: impl
  Future<Output = Result<R, VoiceError>>, undo_room: impl FnOnce(R) -> F) -> Result<(D, R), VoiceError> where
  F: Future<Output = ()>`.

**Why:** journeys V1. `connect` runs token → devices → room → publish in series; the device open (50-300 ms; Sonar
exposes 8 ch at 96 kHz) needs nothing from the room and vice versa. Join #2 spent 515 ms on token + devices.
Expected: −50 to −300 ms per join (the device time hides under the ~1 s room connect), plus the slower of
input/output instead of their sum.

- [ ] **Step 1: Write the failing test**

`mod.rs` `mod tests`:

```rust
    #[tokio::test(start_paused = true)]
    async fn devices_open_while_the_room_connects() {
        let started = tokio::time::Instant::now();
        let r = side_by_side(
            async {
                tokio::time::sleep(Duration::from_millis(200)).await;
                Ok::<_, VoiceError>("devices")
            },
            async {
                tokio::time::sleep(Duration::from_millis(300)).await;
                Ok::<_, VoiceError>("room")
            },
            |_| async {},
        )
        .await;
        assert!(matches!(r, Ok(("devices", "room"))));
        assert_eq!(started.elapsed(), Duration::from_millis(300), "one after the other");
    }

    /// Devices that won't open after the room connected: the room must be closed (dropping a
    /// Room doesn't disconnect it; others would see a ghost).
    #[tokio::test(start_paused = true)]
    async fn a_device_failure_closes_the_connected_room() {
        let closed = Arc::new(AtomicBool::new(false));
        let c = closed.clone();
        let r = side_by_side(
            async { Err::<(), _>(VoiceError::Device("no mic".into())) },
            async { Ok::<_, VoiceError>("room") },
            move |room| async move {
                assert_eq!(room, "room");
                c.store(true, Ordering::SeqCst);
            },
        )
        .await;
        assert!(matches!(r, Err(VoiceError::Device(_))));
        assert!(closed.load(Ordering::SeqCst));
    }

    /// The room fails: the devices opened meanwhile are closed (dropped), not left running.
    #[tokio::test(start_paused = true)]
    async fn a_room_failure_drops_the_opened_devices() {
        struct Io(Arc<AtomicBool>);
        impl Drop for Io {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }
        let dropped = Arc::new(AtomicBool::new(false));
        let r = side_by_side(
            async { Ok::<_, VoiceError>(Io(dropped.clone())) },
            async { Err::<(), _>(VoiceError::Connect("refused".into())) },
            |_| async {},
        )
        .await;
        assert!(matches!(r, Err(VoiceError::Connect(_))));
        assert!(dropped.load(Ordering::SeqCst));
    }
```

- [ ] **Step 2: Run test to verify it fails**

`cargo test -p pulse-client --lib voice::tests::devices_open` → compile error `cannot find function side_by_side`.

- [ ] **Step 3: Write minimal implementation**

`mod.rs`, after `start_audio`:

```rust
/// Open the devices while the room connects (neither needs the other): a join costs the slower
/// of the two, not their sum. If one fails, the other is undone: opened devices are dropped
/// (closing them), a connected room is closed (dropping a Room doesn't disconnect it). Both run
/// to the end: abandoning `Room::connect` midway could leave a half-joined participant.
async fn side_by_side<D, R, F>(
    devices: impl Future<Output = Result<D, VoiceError>>,
    room: impl Future<Output = Result<R, VoiceError>>,
    undo_room: impl FnOnce(R) -> F,
) -> Result<(D, R), VoiceError>
where
    F: Future<Output = ()>,
{
    match tokio::join!(devices, room) {
        (Ok(d), Ok(r)) => Ok((d, r)),
        (Err(e), Ok(r)) => {
            undo_room(r).await;
            Err(e)
        }
        (_, Err(e)) => Err(e), // opened devices (if any) drop here
    }
}
```

`connect`, replacing the serial device open and room connect (`mod.rs:391-405` as they stand after L8/L7a):

```rust
        let devices = async {
            let t = Instant::now();
            let io = match mode {
                AudioMode::Real => start_audio(&cfg, shared.clone(), raw_tx.clone()).await?,
                AudioMode::Null(rate) => AudioIo::start_null(rate, shared.clone(), raw_tx.clone()),
            };
            Ok::<_, VoiceError>((io, ms(t)))
        };
        let room = async {
            let t = Instant::now();
            let (room, events) = tokio::time::timeout(
                CONNECT_TIMEOUT,
                Room::connect(&vt.url, &vt.token, RoomOptions::default()),
            )
            .await
            .map_err(|_| VoiceError::Timeout("connecting to voice"))?
            .map_err(|e| VoiceError::Connect(e.to_string()))?;
            Ok::<_, VoiceError>((room, events, ms(t)))
        };
        let ((io, devices_ms), (room, mut room_events, room_connect_ms)) =
            side_by_side(devices, room, |(room, _, _)| async move {
                let _ = tokio::time::timeout(CLOSE_TIMEOUT, room.close()).await;
            })
            .await?;
        let t = Instant::now();
        tracing::info!("voice: room connected");
```

`devices.rs` `AudioIo::start` (replaces `:999-1010`):

```rust
    pub fn start(cfg: &AudioConfig, shared: Arc<Shared>, raw_tx: RawTx) -> anyhow::Result<Self> {
        // both directions at once: each waits on its own device thread anyway
        let (input, output) = std::thread::scope(|s| {
            let (sh, tx, name) = (shared.clone(), raw_tx.clone(), cfg.input.clone());
            let input = s.spawn(move || open_half(true, name, sh, tx));
            let output = open_half(false, cfg.output.clone(), shared.clone(), raw_tx.clone());
            let input = input
                .join()
                .unwrap_or_else(|_| Err(anyhow!("audio input: opening thread panicked")));
            (input, output)
        });
        Ok(Self {
            input: Some(input?),
            output: Some(output?),
            null: None,
            shared,
            raw_tx,
        })
    }
```

- [ ] **Step 4: Run test to verify it passes**

`cargo test -p pulse-client --lib voice::tests` → pass. `just voice-it` → `rapid_join_leave_never_hangs` and
`leave_cancels_a_stuck_join` (lib) still pass. Measurement: `voice: mic published devices_ms room_connect_ms` and
`voice: joined elapsed_ms`: elapsed should now be ≈ token + max(devices, room) + publish (baseline sum ~1.75 s on
the 160 ms path; expect −50-300 ms).

- [ ] **Step 5: Commit**

`git add client/src-tauri/src/voice/mod.rs client/src-tauri/src/voice/devices.rs`
`git commit -m "perf(voice): open audio devices while the room connects, input and output in parallel"`

---

### Task 19 (L2b): Keep LiveKit's WebRTC runtime alive between calls

**Files:**
- Modify: `client/src-tauri/src/voice/mod.rs:290-318` (`VoiceManager.lk`, `keep_runtime`, `warm_runtime`),
  `connect` (call `keep_runtime` before `Room::connect`), `prefetch` (warm it on hover)
- Test: `client/src-tauri/src/voice/mod.rs` (`mod tests`)

**Interfaces:**
- Consumes: `livekit::rtc_engine::lk_runtime::LkRuntime::instance() -> Arc<LkRuntime>` (pub in livekit 0.9.3;
  held only weakly by the SDK, `lk_runtime.rs:28-35,74-95`).
- Produces: `VoiceManager::keep_runtime(&self)`, `VoiceManager::warm_runtime(&self)` (blocking-pool build).

**Why:** journeys V4. The SDK keeps the PeerConnectionFactory (+ 3 WebRTC threads, synthetic ADM) behind a `Weak`,
so it's destroyed with the last Room at leave and rebuilt on the next join; the first join in the process paid
~400 ms more signal setup than the second (827 vs 420 ms). Expected: −100 to −400 ms on first joins and rejoins
(unproven: measure), and the build moves off the click path when a voice channel is hovered.

- [ ] **Step 1: Write the failing test**

```rust
    /// LiveKit holds its runtime weakly: without our handle it dies with each call's last Room
    /// and the next join rebuilds it (threads, factory: hundreds of ms on the first join).
    #[test]
    fn the_webrtc_runtime_outlives_its_rooms() {
        use livekit::rtc_engine::lk_runtime::LkRuntime;
        let vm = VoiceManager::new(Arc::new(|_| {}));
        vm.keep_runtime();
        let weak = Arc::downgrade(&LkRuntime::instance());
        assert!(weak.upgrade().is_some(), "torn down when its last user went");
    }
```

- [ ] **Step 2: Run test to verify it fails**

`cargo test -p pulse-client --lib voice::tests::the_webrtc_runtime` → compile error `no method named keep_runtime`.

- [ ] **Step 3: Write minimal implementation**

`VoiceManager` field (init `lk: Default::default(),`):

```rust
    /// WebRTC's runtime (peer-connection factory and its threads), kept for the app's life:
    /// LiveKit only holds it weakly, so it was rebuilt on every join after a leave.
    lk: Arc<std::sync::OnceLock<Arc<livekit::rtc_engine::lk_runtime::LkRuntime>>>,
```

```rust
    /// Build WebRTC's runtime if needed, and keep it.
    pub fn keep_runtime(&self) {
        self.lk
            .get_or_init(livekit::rtc_engine::lk_runtime::LkRuntime::instance);
    }

    /// Same, on the blocking pool (it takes tens of ms): for a hover, ahead of the click.
    pub fn warm_runtime(&self) {
        let lk = self.lk.clone();
        tokio::task::spawn_blocking(move || {
            lk.get_or_init(livekit::rtc_engine::lk_runtime::LkRuntime::instance);
        });
    }
```

In `connect`, just before the `let room = async { … }` block: `self.keep_runtime();`. In `prefetch` (L3), after the
token prefetch: `self.warm_runtime();`.

- [ ] **Step 4: Run test to verify it passes**

`cargo test -p pulse-client --lib voice::tests` → pass. Measurement: in one process, join → leave → wait 10 s →
join; compare `room_connect_ms` of the first and second join with the baseline (827 vs 420 ms on the 160 ms
path). Optional: `RUST_LOG=livekit=debug` shows `LkRuntime::new()` only once per process.

- [ ] **Step 5: Commit**

`git add client/src-tauri/src/voice/mod.rs`
`git commit -m "perf(voice): keep WebRTC's runtime between calls; warm it on hover"`

---

### Task 20 (L2c): Show "connected" once the room is connected; publish the mic right after

**Files:**
- Modify: `client/src-tauri/src/voice/mod.rs:234-262` (`VoiceEvent::Mic`), `connect` (publish becomes a session
  task), `:937-985` tests (serialisation)
- Modify: `client/ui/src/lib/voice.svelte.ts:7-30,33-65` (`mic` event, `micLive`)
- Test: `client/src-tauri/src/voice/mod.rs` (`mod tests`), `client/src-tauri/tests/voice_it.rs`

**Interfaces:**
- Consumes: `LocalParticipant::publish_track(LocalTrack, TrackPublishOptions)`, `mic_publish_options()`,
  `PUBLISH_TIMEOUT`, `CLOSE_TIMEOUT`.
- Produces: `VoiceEvent::Mic { live: bool, error: Option<String> }` → `{"kind":"mic","live":…,"error":…}`;
  UI `voice.micLive`. UI state machine check: `transitionSounds` plays `join` on `connecting → connected`
  (`voiceui.ts:24-32`): it now plays ~1 RTT earlier; a later publish failure goes `connected → disconnected`,
  which plays `leave` and shows the error, the right feedback.

**Why:** journeys V5. The UI and join sound wait ~190 ms (1 RTT: AddTrack → TrackPublished) for the publish, but
hearing others doesn't depend on it. Expected: "connected" + join sound ~190 ms sooner; others hear you at the
same time as now.

- [ ] **Step 1: Write the failing test**

`mod.rs` `mod tests`:

```rust
    #[test]
    fn mic_event_serialises_for_the_ui() {
        let e = VoiceEvent::Mic { live: false, error: Some("Couldn't send your mic".into()) };
        assert_eq!(
            serde_json::to_value(&e).unwrap(),
            serde_json::json!({"kind":"mic","live":false,"error":"Couldn't send your mic"})
        );
    }
```

`tests/voice_it.rs` (imports `use pulse_client::voice::{Connection, VoiceEvent};`,
`use std::sync::{Arc, Mutex};`):

```rust
/// "Connected" (and the join sound) must not wait a round trip for the mic publish.
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn connected_shows_before_the_mic_is_published() {
    let mut cfg = testing::test_config();
    cfg.livekit_url = "ws://localhost:7880".into();
    cfg.livekit_secret = DEV_SECRET.into();
    let app = testing::spawn_with(cfg).await;
    let (_, token) = testing::register(&app, "alex").await;
    let s = testing::create_server(&app, &token, "Main").await;
    let api = Api::new(&format!("http://{}", app.addr));
    let lounge = api
        .channels(&token, s.id)
        .await
        .unwrap()
        .into_iter()
        .find(|c| c.name.as_deref() == Some("Lounge"))
        .unwrap();
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let sink = seen.clone();
    let vm = VoiceManager::new(Arc::new(move |e| {
        let tag = match e {
            VoiceEvent::State { connection: Connection::Connected, .. } => "connected".to_string(),
            VoiceEvent::Mic { live, .. } => format!("mic:{live}"),
            _ => return,
        };
        sink.lock().unwrap().push(tag);
    }));
    vm.join(&api, &token, lounge.id, AudioConfig::default(), AudioMode::Null(48_000))
        .await
        .unwrap();
    for _ in 0..100 {
        if seen.lock().unwrap().iter().any(|t| t == "mic:true") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let seen = seen.lock().unwrap().clone();
    let connected = seen.iter().position(|t| t == "connected").expect("connected");
    let live = seen.iter().position(|t| t == "mic:true").expect("mic published");
    assert!(connected < live, "{seen:?}");
    vm.leave().await;
}
```

- [ ] **Step 2: Run test to verify it fails**

`cargo test -p pulse-client --lib voice::tests::mic_event` → compile error `no variant named Mic`.
(`just voice-it` would fail the same way; with the variant stubbed it panics `mic published` — no such event.)

- [ ] **Step 3: Write minimal implementation**

`VoiceEvent` gains:

```rust
    /// Whether our mic is in the room. "Connected" comes first; the publish follows ~1 RTT later.
    /// A failed publish ends the call (`error` says why).
    Mic {
        live: bool,
        error: Option<String>,
    },
```

`connect`: delete the awaited publish block (`published … if let Some(e) = failure { … return Err(e) }`) and the
`voice: mic published` line. After `let room = Arc::new(room);` and `let mut tasks = Vec::new();`, and after
`rx`, `io`, `alive` are created (move their `let`s above this block), add:

```rust
        // publish in the background: "connected" doesn't wait the round trip for it
        {
            let (room, events, controls) = (room.clone(), self.events.clone(), self.controls.clone());
            let (alive, io, rx, shared) = (alive.clone(), io.clone(), rx.clone(), shared.clone());
            tasks.push(tokio::spawn(async move {
                let t = Instant::now();
                let published = tokio::time::timeout(
                    PUBLISH_TIMEOUT,
                    room.local_participant()
                        .publish_track(LocalTrack::Audio(track), mic_publish_options()),
                )
                .await;
                let failure = match published {
                    Ok(Ok(_)) => None,
                    Ok(Err(e)) => Some(e.to_string()),
                    Err(_) => Some("timed out".to_string()),
                };
                let Some(e) = failure else {
                    tracing::info!(publish_ms = ms(t), since_join_ms = ms(started), "voice: mic published");
                    events(VoiceEvent::Mic { live: true, error: None });
                    return;
                };
                // a call nobody can hear us in isn't a call: end it, and say why
                tracing::warn!(error = %e, "voice: publishing the mic failed; leaving");
                alive.store(false, Ordering::SeqCst);
                shared.set_mic_sink(None);
                io.lock().unwrap().take();
                rx.lock().unwrap().clear();
                let _ = tokio::time::timeout(CLOSE_TIMEOUT, room.close()).await;
                events(VoiceEvent::Mic { live: false, error: Some(format!("Couldn't send your mic: {e}")) });
                events(VoiceEvent::State {
                    channel_id: None,
                    connection: Connection::Disconnected,
                    controls: *controls.lock().unwrap(),
                });
            }));
        }
```

and log the join phases at room-connect time:
`tracing::info!(token_ms, token_prefetched, devices_ms, room_connect_ms, "voice: room connected");` (replaces the
bare `voice: room connected`). `join()` keeps emitting `Connected` when `connect` returns, which is now right after
the room connected.

`client/ui/src/lib/voice.svelte.ts`: add `| { kind: 'mic'; live: boolean; error: string | null }` to `VoiceEvent`;
`micLive: false,` (with the doc comment "Our mic is in the room (shows a moment after connected).") to `voice`; in
`case 'state'` add `if (!e.channel_id) voice.micLive = false`; and

```ts
      case 'mic':
        voice.micLive = e.live
        if (e.error) voice.error = e.error
        break
```

- [ ] **Step 4: Run test to verify it passes**

`cargo test -p pulse-client --lib voice::tests` → pass. `just voice-it` (`connected_shows_before_the_mic_is_published`)
→ pass. `cd client/ui && pnpm check && pnpm test` → green. Measurement: `voice: joined elapsed_ms` vs
`voice: mic published since_join_ms`; the gap is the ~190 ms the UI no longer waits.

- [ ] **Step 5: Commit**

`git add client/src-tauri/src/voice/mod.rs client/src-tauri/tests/voice_it.rs client/ui/src/lib/voice.svelte.ts`
`git commit -m "perf(voice): report connected when the room is up; publish the mic right after"`

---

### Task 21 (L4a): Channel switch: the old room closes in the background

**Files:**
- Modify: `client/src-tauri/src/voice/mod.rs:328-369` (`join` detaches instead of leaving), new `detach_locked`
  next to `leave_locked` (`:773-790`)
- Test: `client/src-tauri/tests/voice_it.rs`

**Interfaces:**
- Consumes: `Session { room: Arc<Room>, alive, rx, io, shared, .. }`, `Shared::set_mic_sink` (L8), `CLOSE_TIMEOUT`.
- Produces: `async fn detach_locked(&self)` (private). Event sequence on a switch: `Connected(A)` →
  `Connecting(B)` → `Connected(B)`; no `Disconnected` in between (so no leave sound between rooms).

**Coordination (client-robustness R2, voice auto-rejoin, lands before this task):** R2 adds `epoch`, `last_join` and
`join_inner(.., Some(epoch))` to `voice/mod.rs`. Preserve: (1) a user `join` bumps `epoch` and sets `last_join`
*before* calling `detach_locked`, so a rejoin scheduled for room A is stale the moment B is clicked; (2) `leave`
bumps `epoch` and clears `last_join` (`detach_locked` is not a leave: it touches neither); (3) a rejoin
(`join_inner(.., Some(epoch))`) doesn't announce `Disconnected`, and `detach_locked` must not either. Put the
`detach_locked` call where R2's `join_inner` now calls `leave_locked`.

**Why:** journeys S1. A switch awaits the old `room.close()` (1 RTT, 163 ms observed; up to the 5 s cap on a bad
link) before even asking for the new token. Expected: −160 ms per switch, up to −5 s on lossy links; the UI no
longer flashes Disconnected. For about 1 RTT others may see you in both rooms (our mic already stopped feeding
the old one).

- [ ] **Step 1: Write the failing test**

`tests/voice_it.rs` (imports as in L2c, plus `use pulse_protocol::rest::ChannelKind;`):

```rust
/// A switch goes straight to the new room: no Disconnected flash (leave sound) between rooms,
/// and no waiting for the old room's close.
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn switching_channels_goes_straight_to_the_new_room() {
    let mut cfg = testing::test_config();
    cfg.livekit_url = "ws://localhost:7880".into();
    cfg.livekit_secret = DEV_SECRET.into();
    let app = testing::spawn_with(cfg).await;
    let (_, token) = testing::register(&app, "alex").await;
    let s = testing::create_server(&app, &token, "Main").await;
    let api = Api::new(&format!("http://{}", app.addr));
    let lounge = api
        .channels(&token, s.id)
        .await
        .unwrap()
        .into_iter()
        .find(|c| c.name.as_deref() == Some("Lounge"))
        .unwrap();
    let den = api.create_channel(&token, s.id, ChannelKind::Voice, "Den").await.unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = seen.clone();
    let vm = VoiceManager::new(Arc::new(move |e| {
        if let VoiceEvent::State { channel_id, connection, .. } = e {
            sink.lock().unwrap().push((channel_id, connection));
        }
    }));
    vm.join(&api, &token, lounge.id, AudioConfig::default(), AudioMode::Null(48_000))
        .await
        .unwrap();
    seen.lock().unwrap().clear();
    let t = std::time::Instant::now();
    vm.join(&api, &token, den.id, AudioConfig::default(), AudioMode::Null(48_000))
        .await
        .unwrap();
    println!("switch took {:?}", t.elapsed());
    let states = seen.lock().unwrap().clone();
    assert_eq!(states.first(), Some(&(Some(den.id), Connection::Connecting)), "{states:?}");
    assert!(
        !states.iter().any(|(_, c)| *c == Connection::Disconnected),
        "Disconnected between rooms: {states:?}"
    );
    assert_eq!(vm.current_channel().await, Some(den.id));
    vm.leave().await;
}
```

- [ ] **Step 2: Run test to verify it fails**

`just dev-livekit` then `cargo test -p pulse-client --test voice_it switching_channels -- --ignored --nocapture` →
panics `assertion left == right failed … left: Some((None, Disconnected))` (today's leave emits it first).

- [ ] **Step 3: Write minimal implementation**

`mod.rs`, next to `leave_locked`:

```rust
    /// Leave the current room for another one: stop feeding it, close it in the background
    /// (others may see us in both rooms for about a round trip) and don't report Disconnected
    /// (the switch goes straight to Connecting the next room). A plain Leave still waits.
    async fn detach_locked(&self) {
        let Some(s) = self.session.lock().await.take() else {
            return;
        };
        tracing::info!(channel = %s.channel, "voice: leaving for another channel");
        s.alive.store(false, Ordering::SeqCst);
        s.shared.set_mic_sink(None);
        s.rx.lock().unwrap().clear();
        s.io.lock().unwrap().take();
        let room = s.room.clone();
        tokio::spawn(async move {
            if tokio::time::timeout(CLOSE_TIMEOUT, room.close()).await.is_err() {
                tracing::warn!("voice: old room close timed out; dropping it");
            }
        });
    }
```

In `join` replace `self.leave_locked().await;` (`mod.rs:343`) with `self.detach_locked().await;`. The failure path
of `join` still emits `Disconnected`, so a failed switch ends disconnected as today; app exit and `leave()` keep
the awaited close.

- [ ] **Step 4: Run test to verify it passes**

`just voice-it` → `switching_channels_goes_straight_to_the_new_room`, `rapid_join_leave_never_hangs` and
`receives_peer_audio_at_real_time_across_rejoins` pass. Measurement: switch A → B on the Windows path; the gap
from `voice: leaving for another channel` to `voice: joining` should be ~0 ms (baseline 163 ms).

- [ ] **Step 5: Commit**

`git add client/src-tauri/src/voice/mod.rs client/src-tauri/tests/voice_it.rs`
`git commit -m "perf(voice): close the old room in the background when switching channels"`
**Failure mode covered here: a switch while the old room is reconnecting, or a switch that fails.** LiveKit is in
`Reconnecting` on room A when the user clicks B, and B's join may fail too (same network). The UI must end on
`Disconnected` (join's failure path, since `detach_locked` itself doesn't announce it), our mic must stop
reaching A, and A's close (its own `Disconnected` event) must never pull us back to A, including through R2's
auto-rejoin.

- [ ] **Step 6: Write the failing test**

`voice/processor.rs` `mod tests` (L8 helpers `off`, `collecting`, `wait_frames`):

```rust
    /// A switch hands the mic to the new room: the old one gets nothing more.
    #[test]
    fn a_detached_room_gets_no_more_frames() {
        let sh = off();
        let old = collecting(&sh);
        let (raw_tx, _h) = spawn(sh.clone());
        for _ in 0..5 {
            raw_tx.send((48_000, vec![0.0; 480], Instant::now())).unwrap();
        }
        wait_frames(&old, 5);
        sh.set_mic_sink(None); // detach_locked
        let new = collecting(&sh); // the next room
        let before = old.lock().unwrap().len();
        for _ in 0..5 {
            raw_tx.send((48_000, vec![0.0; 480], Instant::now())).unwrap();
        }
        assert!(wait_frames(&new, 5) >= 5);
        assert_eq!(old.lock().unwrap().len(), before, "the old room kept getting our mic");
    }
```

`tests/voice_it.rs`:

```rust
/// A switch whose join fails ends Disconnected (not stuck "connecting", not back in A), and
/// the old room's close never pulls us back into it.
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn a_failed_switch_ends_disconnected_and_the_old_room_stays_left() {
    let mut cfg = testing::test_config();
    cfg.livekit_url = "ws://localhost:7880".into();
    cfg.livekit_secret = DEV_SECRET.into();
    let app = testing::spawn_with(cfg).await;
    let (_, token) = testing::register(&app, "alex").await;
    let s = testing::create_server(&app, &token, "Main").await;
    let text = testing::general(&app, &token, s.id).await; // no voice: its token request fails
    let api = Api::new(&format!("http://{}", app.addr));
    let lounge = api
        .channels(&token, s.id)
        .await
        .unwrap()
        .into_iter()
        .find(|c| c.name.as_deref() == Some("Lounge"))
        .unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = seen.clone();
    let vm = VoiceManager::new(Arc::new(move |e| {
        if let VoiceEvent::State { channel_id, connection, .. } = e {
            sink.lock().unwrap().push((channel_id, connection));
        }
    }));
    vm.join(&api, &token, lounge.id, AudioConfig::default(), AudioMode::Null(48_000))
        .await
        .unwrap();
    seen.lock().unwrap().clear();
    let r = vm
        .join(&api, &token, text.id, AudioConfig::default(), AudioMode::Null(48_000))
        .await;
    assert!(r.is_err());
    tokio::time::sleep(Duration::from_secs(3)).await; // A's close (and any rejoin) would land by now
    let states = seen.lock().unwrap().clone();
    assert_eq!(states.last(), Some(&(None, Connection::Disconnected)), "{states:?}");
    assert!(
        !states.iter().any(|(c, _)| *c == Some(lounge.id)),
        "pulled back into the old room: {states:?}"
    );
    assert!(!vm.in_voice());
    assert_eq!(vm.current_channel().await, None);
}
```

- [ ] **Step 7: Run test to verify it fails**

`cargo test -p pulse-client --lib voice::processor::tests::a_detached` → passes on L8 code (guard; run once with
`set_mic_sink(None)` removed from the test to see it fail: "the old room kept getting our mic").
`just dev-livekit; cargo test -p pulse-client --test voice_it a_failed_switch -- --ignored --nocapture` → on
Step 3's `detach_locked` (which spawns the close while the session's tasks are still alive) the room-events task
can see A's `Disconnected` and, with R2 in, schedule a rejoin of A: fails with "pulled back into the old room". Without R2 the same race shows as a stray
`(None, Disconnected)` mid-switch in `switching_channels_goes_straight_to_the_new_room` (intermittent).

- [ ] **Step 8: Write minimal implementation**

`detach_locked`: drop the session (aborting its tasks: the room-events task, the stats, the publish) **before**
the close is started, so no `Disconnected` from our own close is ever handled:

```rust
        let room = s.room.clone();
        drop(s); // its tasks stop here: our own close must not look like LiveKit dropping us
        tokio::spawn(async move {
            if tokio::time::timeout(CLOSE_TIMEOUT, room.close()).await.is_err() {
                tracing::warn!("voice: old room close timed out; dropping it");
            }
        });
```

(In L4b, keep the order: clone `s.pipeline` first, then `drop(s)`, then spawn the close.) `join`'s failure path
already emits `State { None, Disconnected }`; keep it after a detach.

- [ ] **Step 9: Run test to verify it passes**

`cargo test -p pulse-client --lib voice::processor` and `just voice-it` (`a_failed_switch_ends_disconnected_and_the_old_room_stays_left`,
`switching_channels_goes_straight_to_the_new_room`) → pass.

- [ ] **Step 10: Commit**

`git add client/src-tauri/src/voice/mod.rs client/src-tauri/src/voice/processor.rs client/src-tauri/tests/voice_it.rs`
`git commit -m "fix(voice): a switch never returns to the old room; a failed switch ends disconnected"`


---

### Task 22 (L4b): Channel switch: keep the audio pipeline and the noise model

**Files:**
- Modify: `client/src-tauri/src/voice/mod.rs`: new `struct Pipeline` (+ `build`, `adopt`, `usable_for`, `Drop`),
  `spawn_watchdog`, `spawn_follow` (bodies moved out of the session's levels task `:658-723` and follow task
  `:724-746`); `Session` (`:266-288`) holds `pipeline: Arc<Pipeline>`; `join`, `connect`, `detach_locked`,
  `leave_locked`, `set_audio_config` (`:880-899`), `diagnostics`, test accessors (`:901-930`); new
  `pipelines_built`
- Test: `client/src-tauri/tests/voice_it.rs`
- cpal: no device API change; `Pipeline::build` calls `start_audio`/`AudioIo::start_null` and the moved
  watchdog/follow code calls `devices::{default_device_names, device_names, on_fallback}` as today.

**Interfaces:**
- Consumes: `Shared::new(cfg, controls, PEER_BUFFER_MS)` (L7a), `processor::spawn(shared)` (L8), `side_by_side`
  (L2a), `MixerInput::clear` (L7a), `Reopener`, `Sides::changed`.
- Produces: `struct Pipeline { shared, cfg, io, reopener, real, alive, tasks }`;
  `Pipeline::build(&AudioConfig, AudioMode, Arc<Mutex<Controls>>, EventSink) -> Result<Pipeline, VoiceError>`;
  `Pipeline::adopt(&self, &AudioConfig) -> Result<(), VoiceError>`; `detach_locked(&self) -> Option<Arc<Pipeline>>`;
  `pub fn pipelines_built(&self) -> u64`; log field `pipeline_reused` on `voice: room connected`.

**Coordination (client-robustness R2, lands before L2a-L4b):** keep R2's `epoch`/`last_join` handling exactly as
in L4a's Coordination line. Here specifically: `connect`'s new `kept: Option<Arc<Pipeline>>` argument is passed
through `join_inner` (a rejoin after LiveKit dropped us gets `kept = None` in practice, since the
`Disconnected` arm marks the pipeline dead, so it rebuilds); the rewritten `leave_locked` must keep R2's epoch
bump + `last_join` clear (wherever R2 put them: in `leave` or `leave_locked`); the rejoin path stays silent
(no `Disconnected` before its `Connecting`).

**Why:** journeys S2, voice V10. Every switch closes and reopens both devices (WASAPI churn, 60-450 ms, Bluetooth
headsets flap) and reloads DeepFilterNet (~300 ms of CPU, during which Standard stands in, the R2ZSZA log shows
~20 reloads in 2 min). With L2a the device open is already off the critical path, so the gain is mostly: Strong
suppression from the first word in the new room, no device glitches, less CPU while connecting; on a switch the
join costs only token + room (−60 to −450 ms if devices were the slower half). The echo canceller also stays
converged.

- [ ] **Step 1: Write the failing test**

`tests/voice_it.rs`:

```rust
/// A switch keeps the devices, the processor and Strong's model; Leave lets them go.
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn switching_keeps_the_audio_pipeline_and_leave_drops_it() {
    let mut cfg = testing::test_config();
    cfg.livekit_url = "ws://localhost:7880".into();
    cfg.livekit_secret = DEV_SECRET.into();
    let app = testing::spawn_with(cfg).await;
    let (_, token) = testing::register(&app, "alex").await;
    let s = testing::create_server(&app, &token, "Main").await;
    let api = Api::new(&format!("http://{}", app.addr));
    let lounge = api
        .channels(&token, s.id)
        .await
        .unwrap()
        .into_iter()
        .find(|c| c.name.as_deref() == Some("Lounge"))
        .unwrap();
    let den = api.create_channel(&token, s.id, ChannelKind::Voice, "Den").await.unwrap();
    let vm = VoiceManager::new(Arc::new(|_| {}));
    let join = |id| vm.join(&api, &token, id, AudioConfig::default(), AudioMode::Null(48_000));
    join(lounge.id).await.unwrap();
    assert_eq!(vm.pipelines_built(), 1);
    join(den.id).await.unwrap();
    assert_eq!(vm.pipelines_built(), 1, "the switch rebuilt the pipeline");
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert!(vm.mixer_pushed().await > 0, "still mixing in the new room");
    vm.leave().await;
    join(lounge.id).await.unwrap();
    assert_eq!(vm.pipelines_built(), 2, "after Leave a join starts fresh");
    vm.leave().await;
}
```

- [ ] **Step 2: Run test to verify it fails**

`cargo test -p pulse-client --test voice_it switching_keeps -- --ignored` → compile error `no method named
pipelines_built`. (With a counter bolted onto today's `connect`, the second assertion fails: `left: 2, right: 1`.)

- [ ] **Step 3: Write minimal implementation**

`mod.rs` (as it stands after L2a-L4a):

```rust
/// What a call needs locally, room or no room: the devices, the mic processor (echo canceller,
/// noise model, gate) and the mixer. Kept across a channel switch, so a switch neither reopens
/// the devices nor reloads Strong's model; dropped on Leave.
struct Pipeline {
    shared: Arc<Shared>,
    cfg: Arc<Mutex<AudioConfig>>,
    io: Arc<Mutex<Option<AudioIo>>>,
    reopener: Arc<Reopener>,
    real: bool,
    /// Cleared when dropped, or when LiveKit dropped us for good (devices already closed).
    alive: Arc<AtomicBool>,
    /// Device watchdog and following: they outlive any one room.
    tasks: Vec<JoinHandle<()>>,
}

impl Drop for Pipeline {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::SeqCst);
        for t in &self.tasks {
            t.abort();
        }
        self.shared.set_mic_sink(None);
        self.io.lock().unwrap().take(); // closes the devices; the processor ends with its senders
    }
}

impl Pipeline {
    async fn build(
        cfg: &AudioConfig,
        mode: AudioMode,
        controls: Arc<Mutex<Controls>>,
        events: EventSink,
    ) -> Result<Self, VoiceError> {
        let shared = Shared::new(cfg, controls, PEER_BUFFER_MS);
        // devices → processor (denoise, gate) → the room's mic sink
        let (raw_tx, _processor) = processor::spawn(shared.clone());
        let io = match mode {
            AudioMode::Real => start_audio(cfg, shared.clone(), raw_tx.clone()).await?,
            AudioMode::Null(rate) => AudioIo::start_null(rate, shared.clone(), raw_tx.clone()),
        };
        let real = matches!(mode, AudioMode::Real);
        let alive = Arc::new(AtomicBool::new(true));
        let io = Arc::new(Mutex::new(Some(io)));
        let cfg = Arc::new(Mutex::new(cfg.clone()));
        let reopener = Arc::new(Reopener {
            cfg: cfg.clone(),
            io: io.clone(),
            shared: shared.clone(),
            raw_tx,
            alive: alive.clone(),
            one_at_a_time: tokio::sync::Mutex::new(()),
            failed: Default::default(),
            count: Default::default(),
        });
        let mut tasks = Vec::new();
        if real {
            tasks.push(spawn_watchdog(shared.clone(), reopener.clone(), events));
            tasks.push(spawn_follow(reopener.clone()));
        }
        Ok(Self { shared, cfg, io, reopener, real, alive, tasks })
    }

    /// New settings (a switch carries the join's settings; or Settings changed): processing
    /// changes live, a changed device reopens only its side.
    async fn adopt(&self, cfg: &AudioConfig) -> Result<(), VoiceError> {
        let changed = Sides::changed(&self.cfg.lock().unwrap(), cfg);
        self.shared.apply_config(cfg);
        *self.cfg.lock().unwrap() = cfg.clone();
        if changed.any() && self.real {
            let targets = (cfg.input.clone(), cfg.output.clone());
            self.reopener.reopen(changed, targets).await?;
        }
        Ok(())
    }

    fn usable_for(&self, mode: AudioMode) -> bool {
        self.alive.load(Ordering::SeqCst)
            && self.real == matches!(mode, AudioMode::Real)
            && self.io.lock().unwrap().is_some()
    }
}
```

The device watchdog and following move out of the session (bodies as today, `mod.rs:701-720` and `:728-745`;
the stall check ran every 5th 100 ms tick, now its own 500 ms tick):

```rust
/// Reopen stalled devices (and say so if that doesn't help). Lives as long as the pipeline.
fn spawn_watchdog(shared: Arc<Shared>, reopener: Arc<Reopener>, events: EventSink) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_millis(500));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut stall = devices::StallPolicy::default();
        loop {
            tick.tick().await;
            if !reopener.alive.load(Ordering::SeqCst) {
                break;
            }
            let now = Instant::now();
            match stall.check(shared.watchdog.stalled(now), now) {
                None => {}
                Some(devices::StallAction::Report) => {
                    tracing::error!("audio device still stalled after reopening");
                    events(VoiceEvent::DeviceStalled);
                }
                Some(devices::StallAction::Recovered) => {
                    tracing::info!("voice: audio device recovered");
                    events(VoiceEvent::DeviceRecovered);
                }
                Some(devices::StallAction::Reopen) => {
                    let mut sides = shared.watchdog.stalled_sides(Instant::now());
                    if !sides.any() {
                        sides = Sides::BOTH; // recovered meanwhile: be thorough anyway
                    }
                    tracing::warn!(?sides, "audio device stalled; reopening");
                    let _ = reopener.reopen(sides, (None, None)).await;
                }
            }
        }
    })
}

/// Follow the system default device / a returning chosen device (own task: enumeration can
/// block, and must not stall the watchdog).
fn spawn_follow(reopener: Arc<Reopener>) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(FOLLOW_EVERY);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let (mut debounce, mut probe) = (devices::Debounce::default(), None);
        loop {
            tick.tick().await;
            if !reopener.alive.load(Ordering::SeqCst) {
                break;
            }
            let switch = reopener.wants_switch(&mut probe).await;
            if debounce.confirm(switch.as_ref().map(|(_, sides, _)| *sides))
                && let Some((reason, sides, targets)) = switch
            {
                tracing::info!(reason, ?sides, "voice: switching devices");
                let _ = reopener.reopen(sides, targets).await;
            }
        }
    })
}
```

The session's levels task keeps only levels, recv-queue sampling, NS status and speaking: delete its `stall`,
`n`, `real` and `match stall.check(…)` parts and the session's follow task.

`Session` becomes `{ channel, room: Arc<Room>, pipeline: Arc<Pipeline>, rx, alive, tasks }` (drop `shared`, `io`,
`cfg`, `real`, `reopener`). `VoiceManager` gains `built: std::sync::atomic::AtomicU64` (init `Default::default()`)
and

```rust
    /// Pipelines built by this manager (tests: a switch must reuse the one it has).
    pub fn pipelines_built(&self) -> u64 {
        self.built.load(Ordering::Relaxed)
    }
```

`detach_locked` returns what to keep:

```rust
    async fn detach_locked(&self) -> Option<Arc<Pipeline>> {
        let s = self.session.lock().await.take()?;
        tracing::info!(channel = %s.channel, "voice: leaving for another channel");
        s.alive.store(false, Ordering::SeqCst);
        s.pipeline.shared.set_mic_sink(None);
        s.rx.lock().unwrap().clear();
        let (room, pipeline) = (s.room.clone(), s.pipeline.clone());
        drop(s); // its tasks stop here: our own close must not look like LiveKit dropping us (L4a Step 8)
        tokio::spawn(async move {
            if tokio::time::timeout(CLOSE_TIMEOUT, room.close()).await.is_err() {
                tracing::warn!("voice: old room close timed out; dropping it");
            }
        });
        Some(pipeline)
    }
```

`join`: `let kept = self.detach_locked().await;` and pass `kept` as a new last argument to `connect` (a cancelled
or failed join drops it, closing the devices). `connect(…, kept: Option<Arc<Pipeline>>)`: after the token,

```rust
        self.keep_runtime();
        let reuse = kept.filter(|p| p.usable_for(mode));
        let pipeline_reused = reuse.is_some();
        if !pipeline_reused {
            self.built.fetch_add(1, Ordering::Relaxed);
        }
        let (controls, events) = (self.controls.clone(), self.events.clone());
        let pipeline = async move {
            let t = Instant::now();
            let p = match reuse {
                Some(p) => {
                    if let Err(e) = p.adopt(&cfg).await {
                        tracing::warn!(error = %e, "voice: the join's devices didn't open; kept the current ones");
                    }
                    p
                }
                None => Arc::new(Pipeline::build(&cfg, mode, controls, events).await?),
            };
            Ok::<_, VoiceError>((p, ms(t)))
        };
```

then `side_by_side(pipeline, room, …)` in place of `side_by_side(devices, room, …)` (L2a), yielding
`(pipeline, devices_ms)`; right after: `let shared = pipeline.shared.clone(); shared.mix_in.clear();` (the last
room's peers) and add `pipeline_reused` to the `voice: room connected` fields. Delete the per-session
`Shared::new`, `processor::spawn`, `io`, `cfg` and `Reopener` construction from `connect`. The `Disconnected` arm
of the room-events task captures `let (pl_alive, io) = (pipeline.alive.clone(), pipeline.io.clone());` and does
`pl_alive.store(false, …); io.lock().unwrap().take();` (next join builds a fresh pipeline); the L2c publish-failure
path does the same. `Session { …, pipeline, … }` at the end.

`leave_locked` (plain Leave still waits for the close, then drops the pipeline with the session):

```rust
    async fn leave_locked(&self) {
        let s = self.session.lock().await.take();
        if let Some(s) = s {
            tracing::info!("voice: leaving");
            s.alive.store(false, Ordering::SeqCst);
            s.pipeline.shared.set_mic_sink(None);
            if tokio::time::timeout(CLOSE_TIMEOUT, s.room.close()).await.is_err() {
                tracing::warn!("voice: room close timed out; dropping it");
            }
            s.rx.lock().unwrap().clear();
            drop(s); // the last handle on the pipeline: devices close, the processor ends
            self.emit_state(None, Connection::Disconnected);
        }
    }
```

`set_audio_config`:

```rust
    pub async fn set_audio_config(&self, cfg: AudioConfig) -> Result<(), VoiceError> {
        let Some(p) = self.session.lock().await.as_ref().map(|s| s.pipeline.clone()) else {
            return Ok(());
        };
        // not under the session lock: opening can take seconds, and Leave, mute and volume
        // must not wait on it (the reopener takes turns and checks `alive`)
        p.adopt(&cfg).await
    }
```

Accessors: `diagnostics` reads `s.pipeline.cfg`; `set_peer_volume` → `s.pipeline.shared.mix_in.set_gain(…)`;
`mixer_pushed` → `s.pipeline.shared.mix_in.pushed_total()`; `send_queue_ms` → `s.pipeline.shared.mic_queue.current()`;
`reopens` → `s.pipeline.reopener.count.load(…)`.

- [ ] **Step 4: Run test to verify it passes**

`cargo test -p pulse-client --lib voice` and `just voice-it` → all pass, including
`switching_keeps_the_audio_pipeline_and_leave_drops_it`, `reopening_one_side_leaves_the_other_alone` and
`real_devices_are_not_reopened_without_a_reason` (run those two on a machine with devices). Measurement: switch
A → B with Strong on: no `audio input opened`/`audio output opened` and no DeepFilterNet load lines after the
switch; `voice: room connected pipeline_reused=true devices_ms≈0`; `voice: path` shows Strong active in the first
window of room B.

- [ ] **Step 5: Commit**

`git add client/src-tauri/src/voice/mod.rs client/src-tauri/tests/voice_it.rs`
`git commit -m "perf(voice): keep devices, processor and noise model across a channel switch"`

---

---

## Phase 6: Footprint

### Task 23 (F3): The UI store holds the reducer's output raw (`$state.raw`), not as a deep proxy

**Why:** opt-client.md H2. `store.svelte.ts:11` wraps the whole `AppState` in a deep `$state` proxy. Every
`app.state = …` builds a new proxy tree, lazily creates a signal and a child proxy per property read, and the reducer
runs its spreads through proxy traps. I checked the reducer: it is already immutable. A grep found no in-place writes
to `app.state.*`, and the only `.sort()` (`state.ts:87`) runs on a fresh copy. Holding the output raw keeps the
identity of unchanged slices, so deriveds over them stop there. Expected: scripting per gateway event drops from
milliseconds to tens of µs, and the "Proxy" heap goes from O(messages × props) to 0.

**Files:**
- Modify: `client/ui/src/lib/store.svelte.ts` (line 11)
- Modify: `client/ui/vite.config.ts` (add `resolve` for tests)
- Create: `client/ui/src/lib/store.svelte.test.ts`

**Interfaces:**
- Consumes: `state.ts` (`emptyState`, `applyEvent`).
- Produces: `app` with the same shape (`app.state`, read and assigned exactly as today), now backed by a raw field.
  Also the vitest `browser` resolve condition, which F6's component test relies on.

- [ ] **Step 1: Write the failing test**

`client/ui/src/lib/store.svelte.test.ts`:

```ts
// @vitest-environment jsdom
import { flushSync } from 'svelte'
import { describe, expect, it } from 'vitest'
import { app } from './store.svelte'
import { applyEvent, emptyState } from './state'

describe('app store', () => {
  it('holds the reducer output as-is (no deep proxy)', () => {
    const s = emptyState()
    app.state = s
    expect(app.state).toBe(s)
    expect(app.state.channels).toBe(s.channels)
  })

  it('still re-runs effects when a new state is assigned', () => {
    app.state = emptyState()
    let runs = 0
    const stop = $effect.root(() => {
      $effect(() => {
        void app.state.conn
        runs++
      })
    })
    flushSync()
    expect(runs).toBe(1)
    app.state = { ...app.state, conn: 'reconnecting' }
    flushSync()
    expect(runs).toBe(2)
    stop()
  })

  it('an unchanged slice keeps its identity across events, so slice deriveds stop there', () => {
    app.state = emptyState()
    const people = app.state.people
    app.state = applyEvent(app.state, { t: 'Typing', d: { channel_id: 'c1', user_id: 'u1' } } as never, 0)
    expect(app.state.people).toBe(people)
  })
})
```

In `client/ui/vite.config.ts`, add after `server: { port: 1420, strictPort: true },`:

```ts
  // Tests: resolve Svelte's client runtime, so $effect and mount() work under vitest (the default is the SSR build).
  resolve: process.env.VITEST ? { conditions: ['browser'] } : undefined,
```

Without this line the effect test fails with `runs = 0` for the wrong reason.

- [ ] **Step 2: Run the test to verify it fails**

`cd client/ui && pnpm vitest run src/lib/store.svelte.test.ts`. Expected: 1 failed, 2 passed. The failure is "holds
the reducer output as-is" (`expected {…} to be {…} // Object.is equality`): today `app.state` hands back a proxy, not
`s`. The other existing tests still pass with the new resolve condition: 95 passed.

- [ ] **Step 3: Write the minimal implementation**

In `client/ui/src/lib/store.svelte.ts`, replace line 11

```ts
export const app = $state<{ state: AppState }>({ state: emptyState() })
```

with

```ts
// The reducer (state.ts) is pure and never mutates: hold its output as-is (`$state.raw`) instead
// of deep-proxying it. Reassigning `app.state` is the only way to change it, and the only way that
// re-renders; unchanged slices keep their identity, so deriveds over them stop there.
class AppStore {
  state = $state.raw<AppState>(emptyState())
}
export const app = new AppStore()
```

A class field is used because a `$state.raw` object literal field isn't allowed, and an exported `let` can't be
reassigned from other modules. Every call site (`app.state = …`, `app.state.x`) stays as it is.

- [ ] **Step 4: Run the tests to verify they pass**

`cd client/ui && pnpm test && pnpm check`. Expected: all pass (96), and svelte-check reports 0 errors, 0 warnings.

Manual check on a dev build (`just dev-client`):
- Open a channel with 200+ messages.
- Have a second profile (`PULSE_PROFILE=b`) type for 10 s.
- Take a DevTools Performance trace, and a heap snapshot filtered on "Proxy".

Expected: the per-event scripting time on `pulse://event` is visibly lower, and no `Proxy` objects hold messages.

- [ ] **Step 5: Commit**

```bash
git add client/ui/src/lib/store.svelte.ts client/ui/src/lib/store.svelte.test.ts client/ui/vite.config.ts
git commit -m "perf(ui): hold app state raw instead of a deep proxy"
```

---

### Task 24 (F4): Cache rendered markdown per message and keep the mention list stable

**Why:** opt-client.md H1. `MessageItem.svelte:80` runs `renderMarkdown` (marked → DOMPurify → template re-parse) for
every loaded message whenever `mentionNames` gets a new identity. `TextChannel.svelte:95-102` rebuilds it on every
event: Typing every 3 s per typer, presence, voice, and other channels' messages. With 200 messages that is about
10–60 ms of main-thread work per event. It happens again for every message on each channel revisit (`{#key}` remount
in `Shell.svelte:296`). Expected: about 0 `renderMarkdown` calls per Typing event, and close to free revisits.

**Files:**
- Modify: `client/ui/src/lib/markdown.ts` (new `renderMessage`, `RENDER_CACHE_MAX`; import `Message`)
- Modify: `client/ui/src/lib/selectors.ts` (new `stableList`)
- Modify: `client/ui/src/components/MessageItem.svelte` (line 3 import; lines 79–80)
- Modify: `client/ui/src/components/TextChannel.svelte` (line 11 import; line 102)
- Test: `client/ui/src/lib/markdown.test.ts`, `client/ui/src/lib/selectors.test.ts`

**Interfaces:**
- Consumes: `renderMarkdown(src, mentionNames)` (unchanged, still the only sanitiser).
- Produces: `renderMessage(m: Pick<Message,'id'|'edited_at'|'content'>, mentionNames?: string[]): string` and
  `stableList<T>(prev: T[], next: T[]): T[]`.

- [ ] **Step 1: Write the failing test**

Append to `client/ui/src/lib/markdown.test.ts`, and add `import DOMPurify from 'dompurify'` and `vi` to the imports at
the top (`import { describe, expect, it, vi } from 'vitest'`, `import { RENDER_CACHE_MAX, renderMarkdown,
renderMessage, typingThrottle } from './markdown'`):

```ts
describe('renderMessage (cached)', () => {
  const msg = (id: string, content: string, edited_at: string | null = null) => ({ id, content, edited_at })

  it('renders each message once, however often it is shown', () => {
    const spy = vi.spyOn(DOMPurify, 'sanitize')
    const names = ['sam']
    const a = renderMessage(msg('01A', 'hi **@sam**'), names)
    const b = renderMessage(msg('01A', 'hi **@sam**'), names)
    expect(b).toBe(a)
    expect(spy).toHaveBeenCalledTimes(1)
    renderMessage(msg('01A', 'hi **@sam**'), ['sam']) // equal names, new array: still cached
    expect(spy).toHaveBeenCalledTimes(1)
    spy.mockRestore()
  })

  it('an edit or different mentionable names render again', () => {
    expect(renderMessage(msg('01B', 'old'))).toContain('old')
    expect(renderMessage(msg('01B', 'new', '2026-10-16T08:00:00Z'))).toContain('new')
    expect(renderMessage(msg('01C', '@sam'), [])).not.toContain('mention')
    expect(renderMessage(msg('01C', '@sam'), ['sam'])).toContain('<span class="mention">@sam</span>')
  })

  it('is bounded: the least recently shown entries are evicted', () => {
    renderMessage(msg('KEEP', 'kept'))
    for (let i = 0; i < RENDER_CACHE_MAX; i++) {
      renderMessage(msg(`F${i}`, 'x'))
      if (i % 100 === 0) renderMessage(msg('KEEP', 'kept')) // shown now and then: stays
    }
    const spy = vi.spyOn(DOMPurify, 'sanitize')
    renderMessage(msg('KEEP', 'kept'))
    expect(spy).not.toHaveBeenCalled()
    renderMessage(msg('F0', 'x'))
    expect(spy).toHaveBeenCalledTimes(1)
    spy.mockRestore()
  })
})
```

Append to `client/ui/src/lib/selectors.test.ts`:

```ts
describe('stableList', () => {
  it('keeps the old array when nothing changed', async () => {
    const { stableList } = await import('./selectors')
    const a = ['sam', 'alex']
    expect(stableList(a, ['sam', 'alex'])).toBe(a)
    const b = ['sam']
    expect(stableList(a, b)).toBe(b)
    expect(stableList(a, ['alex', 'sam'])).not.toBe(a)
  })
})
```

- [ ] **Step 2: Run the tests to verify they fail**

`cd client/ui && pnpm vitest run src/lib/markdown.test.ts src/lib/selectors.test.ts`. Expected: they fail on the
missing exports (`RENDER_CACHE_MAX` / `renderMessage` are not exported; `stableList is not a function`).

- [ ] **Step 3: Write the minimal implementation**

`client/ui/src/lib/markdown.ts`: add `import type { Message } from './protocol/Message'` after the `marked` import.
Then insert before `const MENTION = /…/g`:

```ts
/** Most rendered messages kept (a few long channels' worth); the least recently shown go first. */
export const RENDER_CACHE_MAX = 2000
const rendered = new Map<string, string>()
const namesKeys = new WeakMap<string[], string>()

/** `renderMarkdown` for a message, cached: a re-render (someone typing, presence, a channel
 * revisit) reuses the HTML instead of re-running marked + DOMPurify. An edit changes `edited_at`,
 * and a different set of mentionable names is a different entry, so neither can show stale HTML. */
export function renderMessage(m: Pick<Message, 'id' | 'edited_at' | 'content'>, mentionNames: string[] = []): string {
  let names = namesKeys.get(mentionNames)
  if (names === undefined) {
    names = mentionNames.join('\u0000')
    namesKeys.set(mentionNames, names)
  }
  const key = `${m.id}\u0001${m.edited_at ?? ''}\u0001${names}`
  const hit = rendered.get(key)
  if (hit !== undefined) {
    rendered.delete(key) // re-insert: Map order is the LRU order
    rendered.set(key, hit)
    return hit
  }
  const html = renderMarkdown(m.content, mentionNames)
  rendered.set(key, html)
  if (rendered.size > RENDER_CACHE_MAX) rendered.delete(rendered.keys().next().value!)
  return html
}
```

`client/ui/src/lib/selectors.ts`: append

```ts
/** `prev` when `next` has the same items in the same order, else `next`: keeps a derived list's
 * identity across events that didn't change it, so components fed with it don't re-render. */
export function stableList<T>(prev: T[], next: T[]): T[] {
  return prev.length === next.length && prev.every((v, i) => v === next[i]) ? prev : next
}
```

`client/ui/src/components/MessageItem.svelte`:
- Line 3: change `import { renderMarkdown } from '../lib/markdown'` to `import { renderMessage } from '../lib/markdown'`.
- Lines 79–80: replace

  ```svelte
            <!-- renderMarkdown output is DOMPurify-sanitised (allow-list); the only {@html} in the app. -->
            <div class="content">{@html renderMarkdown(message.content, mentionNames)}</div>
  ```

  with

  ```svelte
            <!-- renderMessage output is DOMPurify-sanitised (allow-list, cached per message); the only {@html} in the app. -->
            <div class="content">{@html renderMessage(message, mentionNames)}</div>
  ```

`client/ui/src/components/TextChannel.svelte`:
- Line 11: change to `import { displayName, stableList, typingNames } from '../lib/selectors'`.
- Line 102: replace `const mentionNames = $derived(mentionables.map((p) => p.name))` with

  ```ts
    // Same names → same array, so an unrelated event (typing, presence) doesn't re-render every message.
    let lastNames: string[] = []
    const mentionNames = $derived((lastNames = stableList(lastNames, mentionables.map((p) => p.name))))
  ```

- [ ] **Step 4: Run the tests to verify they pass**

`cd client/ui && pnpm test && pnpm check && pnpm build`: all green.

Manual check: in a dev build, put a temporary `console.count('render')` in `renderMarkdown`, then have a second
profile type in the open channel. Expected: the count doesn't move per Typing event, and revisiting a channel adds
nothing. Remove the counter afterwards.

- [ ] **Step 5: Commit**

```bash
git add client/ui/src/lib/markdown.ts client/ui/src/lib/markdown.test.ts client/ui/src/lib/selectors.ts client/ui/src/lib/selectors.test.ts client/ui/src/components/MessageItem.svelte client/ui/src/components/TextChannel.svelte
git commit -m "perf(ui): cache rendered markdown per message; stable mention list"
```

---

### Task 25 (F5): Trim a channel's history to its newest 100 messages when you leave it

**Why:** opt-client.md M6. `messages[channel]` only ever grows: `addHistory` pages older messages in without a limit,
and `applyReady` keeps them across reconnects. A long-running app that has scrolled back in a few channels holds
thousands of messages, plus their DOM on every revisit.

How loading older messages works today:
- `TextChannel.maybeLoadOlder` (lines 78–92) runs at `scrollTop ≤ 60`.
- It calls `listMessages(channel, before = messages[0].id)` and gets pages of 50.
- `history[c].start` is set when a page has fewer than 50 messages, and that stops further loads.

The cap is **100 (two pages), applied on leaving a channel**. The open channel is never trimmed, so nothing moves
under the reader. Trimming clears `start`, so scrolling up there fetches older pages again through the existing path.
Expected: a bounded heap and DOM. A revisit renders at most 100 messages, and older ones come back on demand.

**Files:**
- Modify: `client/ui/src/lib/state.ts` (before `addHistory`, line 242: new `KEEP_ON_LEAVE`, `trimHistory`, `addOlder`)
- Modify: `client/ui/src/components/TextChannel.svelte`:
  - line 10 import;
  - line 75: a null-safe `scrollTo` after an unmount;
  - lines 83–86 in `maybeLoadOlder`.
- Modify: `client/ui/src/routes/Shell.svelte` (line 14 and line 18 imports; after the effect at lines 77–79)
- Test: `client/ui/src/lib/state.test.ts`; Create `client/ui/src/components/TextChannel.test.ts`

The trim never drops unread messages, and it was chosen over "clamp the recount to the server's count" for two
reasons:
- `ReadStateUpdated` carries only the read point (`state.ts:209-224` recounts from loaded messages). So there is no
  server count to clamp to, short of changing the protocol.
- The NEW line (`firstUnreadIndex`) needs the unread messages loaded anyway.

The cost is that a channel left with many unread messages keeps them until you read it. That is bounded by what was
loaded, and it is the case where you want them.

**Interfaces:**
- Consumes: `addHistory`, `markHistory`.
- Produces: `KEEP_ON_LEAVE = 100`, `trimHistory(s, channel, keep?)` (never cuts at or after the first unread
  message), and `addOlder(s, channel, before, older)` (drops a page that was requested before a trim).

- [ ] **Step 1: Write the failing test**

Append to `client/ui/src/lib/state.test.ts`:

```ts
describe('trimming history on leave', () => {
  const page = (from: number, n: number) =>
    Array.from({ length: n }, (_, i) => msg(`M${String(from + i).padStart(4, '0')}`))

  it('keeps the newest KEEP_ON_LEAVE messages and makes older ones fetchable again', async () => {
    const { addHistory, markHistory, trimHistory, KEEP_ON_LEAVE } = await import('./state')
    let s = applyReady(emptyState(), ready())
    s = markHistory(addHistory(s, 'C1', page(0, 300)), 'C1', true)
    const t = trimHistory(s, 'C1')
    expect(t.messages['C1']).toHaveLength(KEEP_ON_LEAVE)
    expect(t.messages['C1'][0].id).toBe('M0200')
    expect(t.messages['C1'].at(-1)!.id).toBe('M0299')
    expect(t.history['C1']).toEqual({ loaded: true, start: false })
  })

  it('leaves short channels (and the state object) alone', async () => {
    const { addHistory, trimHistory } = await import('./state')
    const s = addHistory(applyReady(emptyState(), ready()), 'C1', page(0, 40))
    expect(trimHistory(s, 'C1')).toBe(s)
    expect(trimHistory(s, 'nope')).toBe(s)
  })

  it('an older page that lands after a trim is dropped instead of leaving a hole', async () => {
    const { addHistory, addOlder, trimHistory } = await import('./state')
    let s = addHistory(applyReady(emptyState(), ready()), 'C1', page(100, 200)) // M0100..M0299
    const before = s.messages['C1'][0].id // scroll-up asks for messages before M0100
    s = trimHistory(s, 'C1') // you switched away meanwhile: M0200..M0299 remain
    const late = addOlder(s, 'C1', before, page(50, 50)) // M0050..M0099 arrives
    expect(late).toBe(s)
    const fresh = addOlder(s, 'C1', 'M0200', page(150, 50)) // a page for the current head is added
    expect(fresh.messages['C1'][0].id).toBe('M0150')
    expect(fresh.messages['C1']).toHaveLength(150)
  })
})

describe('trimming keeps unread messages', () => {
  const page = (from: number, n: number) =>
    Array.from({ length: n }, (_, i) => msg(`M${String(from + i).padStart(4, '0')}`))
  const readAt = (point: string | null) =>
    ({ t: 'ReadStateUpdated', d: { user_id: 'U1', channel_id: 'C1', last_read_message_id: point } }) as const

  it('never trims past your read point, so unread counts survive a trim', async () => {
    const { addHistory, trimHistory } = await import('./state')
    let s = addHistory(applyReady(emptyState(), ready()), 'C1', page(0, 300)) // M0000..M0299
    s = applyEvent(s, readAt('M0049'), 0) // 250 unread
    expect(s.reads['C1'].unread).toBe(250)
    const t = trimHistory(s, 'C1')
    expect(t.messages['C1'][0].id).toBe('M0050') // the oldest unread is kept
    expect(applyEvent(t, readAt('M0049'), 0).reads['C1'].unread).toBe(250)
  })

  it('a channel you have never read is not trimmed at all', async () => {
    const { addHistory, trimHistory } = await import('./state')
    let s = addHistory(applyReady(emptyState(), ready()), 'C1', page(0, 300))
    s = applyEvent(s, readAt(null), 0)
    expect(trimHistory(s, 'C1')).toBe(s)
  })

  it('a read channel still trims to KEEP_ON_LEAVE', async () => {
    const { addHistory, trimHistory, KEEP_ON_LEAVE } = await import('./state')
    let s = addHistory(applyReady(emptyState(), ready()), 'C1', page(0, 300))
    s = applyEvent(s, readAt('M0299'), 0)
    expect(trimHistory(s, 'C1').messages['C1']).toHaveLength(KEEP_ON_LEAVE)
  })
})
```

Create `client/ui/src/components/TextChannel.test.ts`. This is the "leave while an older page is loading" case, and
it needs F3's `resolve.conditions`:

```ts
// @vitest-environment jsdom
import { flushSync, mount, tick, unmount } from 'svelte'
import { describe, expect, it, vi } from 'vitest'
import type { Message } from '../lib/protocol/Message'

// The core: list_messages for the scroll-up page waits until the test releases it.
let releaseOlder: (page: Message[]) => void = () => {}
const invoke = vi.fn(async (cmd: string, args?: { before?: string | null }): Promise<unknown> => {
  if (cmd === 'list_messages' && args?.before) return new Promise((done) => (releaseOlder = done))
  return undefined
})
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))

import TextChannel from './TextChannel.svelte'
import { app } from '../lib/store.svelte'
import { addHistory, applyReady, emptyState, markHistory, trimHistory } from '../lib/state'

const msg = (n: number): Message => ({
  id: `M${String(n).padStart(4, '0')}`, channel_id: 'C1', author_id: 'U2', kind: 'normal', content: `m${n}`,
  reply_to_id: null, created_at: '2026-10-01T00:00:00.000Z', edited_at: null, deleted: false, mentions: [],
})
const page = (from: number, n: number) => Array.from({ length: n }, (_, i) => msg(from + i))

describe('TextChannel: leaving while an older page is loading', () => {
  it('drops the late page (no hole) and does not throw', async () => {
    const me = { id: 'U1', username: 'alex', avatar_hash: null }
    const s = applyReady(emptyState(), {
      me, servers: [{ id: 'S1', name: 'Main', icon_hash: null }],
      channels: [{ id: 'C1', server_id: 'S1', kind: 'text', name: 'general', position: 0 }],
      members: [{ server_id: 'S1', members: [{ user: me, nickname: null }] }],
      dm_members: [], voice: [], people: [], read_states: [], mutes: [], hidden: [], latest: [],
    })
    app.state = markHistory(addHistory(s, 'C1', page(100, 200)), 'C1', false) // M0100..M0299
    const c = mount(TextChannel, { target: document.body, props: { channelId: 'C1', serverId: 'S1' } })
    flushSync()
    document.querySelector('.list')!.dispatchEvent(new Event('scroll')) // scrollTop 0: load older
    expect(invoke).toHaveBeenCalledWith('list_messages', { channelId: 'C1', before: 'M0100' })

    unmount(c) // switched channel: Shell trims on leave
    app.state = trimHistory(app.state, 'C1')
    const trimmed = app.state.messages['C1']
    releaseOlder(page(50, 50)) // M0050..M0099 lands late
    await tick()
    await new Promise((r) => setTimeout(r, 0))

    expect(app.state.messages['C1']).toBe(trimmed)
    expect(app.state.messages['C1'][0].id).toBe('M0200')
  })
})
```

- [ ] **Step 2: Run the tests to verify they fail**

- `cd client/ui && pnpm vitest run src/lib/state.test.ts`: the 6 new tests fail (`trimHistory is not a function`,
  `addOlder is not a function`).
- `pnpm vitest run src/components/TextChannel.test.ts`: fails. The run reports an unhandled rejection
  `TypeError: Cannot read properties of null (reading 'scrollTo')` from `TextChannel.svelte:75`: the stick-to-bottom
  `tick().then(() => list.scrollTo(…))` runs after the unmount. That is an existing crash on fast channel switches.
  Once that is fixed and only `addOlder` is missing, it fails with `expected [ { id: 'M0050' … } ] to be [ { id:
  'M0200' … } ]`, which is the hole. I checked both.

- [ ] **Step 3: Write the minimal implementation**

`client/ui/src/lib/state.ts`, insert before `/** Prepend an older page of history (from scroll-up). */`:

```ts
/** Messages kept for a channel when you leave it: its two newest pages. */
export const KEEP_ON_LEAVE = 100

/** Keep only the newest `keep` messages of `channel` (on leaving it), so a long session doesn't
 * hold every message ever scrolled through. Older ones become fetchable again on scroll-up. */
export function trimHistory(s: AppState, channel: string, keep = KEEP_ON_LEAVE): AppState {
  const list = s.messages[channel]
  if (!list || list.length <= keep) return s
  // Never drop unread messages: ReadStateUpdated recounts unread from what's loaded, and the NEW
  // line needs them. A channel with a read state but no read point is all unread: keep it whole.
  const read = s.reads[channel]
  const firstUnread = !read ? -1 : read.last_read_message_id === null ? 0 : list.findIndex((m) => m.id > read.last_read_message_id!)
  const cut = firstUnread >= 0 ? Math.min(list.length - keep, firstUnread) : list.length - keep
  if (cut <= 0) return s
  const h = s.history[channel]
  return {
    ...s,
    messages: { ...s.messages, [channel]: list.slice(cut) },
    history: h ? { ...s.history, [channel]: { ...h, start: false } } : s.history,
  }
}

/** Add an older page fetched with `before` = the channel's oldest message at the time. Dropped if
 * the list was trimmed meanwhile: adding it would leave a hole that scroll-up never fills. */
export function addOlder(s: AppState, channel: string, before: string, older: Message[]): AppState {
  if (s.messages[channel]?.[0]?.id !== before) return s
  return addHistory(s, channel, older)
}
```

`client/ui/src/components/TextChannel.svelte`:
- Line 10: change to `import { addHistory, addOlder, addPending, markHistory } from '../lib/state'`.
- Line 75: change `if (nearBottom) tick().then(() => list.scrollTo({ top: list.scrollHeight }))` to
  `if (nearBottom) tick().then(() => list?.scrollTo({ top: list.scrollHeight })) // gone if we unmounted meanwhile`.
- In `maybeLoadOlder`, change `      list.scrollTop = list.scrollHeight - before // keep the view where it was` to
  `      if (list) list.scrollTop = list.scrollHeight - before // keep the view where it was (unless we left)`.
- In `maybeLoadOlder`, replace

  ```ts
        const page = await api.listMessages(channelId, messages[0].id)
        app.state = markHistory(addHistory(app.state, channelId, page), channelId, page.length < 50)
  ```

  with

  ```ts
        const oldest = messages[0].id
        const page = await api.listMessages(channelId, oldest)
        app.state = markHistory(addOlder(app.state, channelId, oldest, page), channelId, page.length < 50)
  ```

`client/ui/src/routes/Shell.svelte`:
- Line 14: change to `import { onMount, untrack } from 'svelte'`.
- Line 18: change to `import { applyEvent, trimHistory } from '../lib/state'`.
- After the effect `$effect(() => { ui.openChannelId = activeChannelId })`, add

  ```ts
    // Leaving a channel trims it to its newest messages (scrolling up there fetches older ones again).
    $effect(() => {
      const left = activeChannelId
      return () => {
        if (left) untrack(() => (app.state = trimHistory(app.state, left)))
      }
    })
  ```

- [ ] **Step 4: Run the tests to verify they pass**

`cd client/ui && pnpm test && pnpm check`: all green (state tests + the `TextChannel` test, no unhandled errors).

Manual check in a dev build:
1. In a channel, scroll up past 300 messages, then switch away and back.
2. Expected: the list starts at the newest 100 and the view sits at the bottom. Scrolling up loads older pages again
   with the scroll position kept, and "This is the start of #…" only shows once you really reach the start.

- [ ] **Step 5: Commit**

```bash
git add client/ui/src/lib/state.ts client/ui/src/lib/state.test.ts client/ui/src/components/TextChannel.svelte client/ui/src/components/TextChannel.test.ts client/ui/src/routes/Shell.svelte
git commit -m "perf(ui): trim a channel's history to its newest 100 messages when you leave it"
```

---

### Task 26 (F10): The typing indicator uses one timer for the next expiry, not a 1 s interval per open channel

**Why:** opt-client.md M5. `TextChannel.svelte:39-42` runs `setInterval(…, 1000)` whenever any text channel is open,
which is almost always. That is 3,600 renderer wakeups an hour plus a Svelte flush, and it keeps Chromium's timer
throttling from settling. It only feeds `typers`. Expected: 0 idle wakeups from this source. The indicator still
clears on time (within 50 ms of expiry instead of within 1 s).

**Files:**
- Modify: `client/ui/src/lib/selectors.ts` (new `nextTypingExpiry`)
- Modify: `client/ui/src/components/TextChannel.svelte` (line 11 import; lines 38–42)
- Test: `client/ui/src/lib/selectors.test.ts`

**Interfaces:** Consumes `AppState.typing` (`channelId → userId → expiresAt ms`). Produces
`nextTypingExpiry(s, channelId, now): number | null`.

- [ ] **Step 1: Write the failing test**

Append to `client/ui/src/lib/selectors.test.ts`:

```ts
describe('nextTypingExpiry', () => {
  it('is the earliest live expiry, or null when nobody is typing', async () => {
    const { nextTypingExpiry } = await import('./selectors')
    let t = applyEvent(applyReady(emptyState(), r), { t: 'Typing', d: { channel_id: 'C1', user_id: 'U2' } }, 1000)
    t = applyEvent(t, { t: 'Typing', d: { channel_id: 'C1', user_id: 'U1' } }, 3000)
    expect(nextTypingExpiry(t, 'C1', 2000)).toBe(7000)
    expect(nextTypingExpiry(t, 'C1', 7000)).toBe(9000)
    expect(nextTypingExpiry(t, 'C1', 9000)).toBeNull()
    expect(nextTypingExpiry(t, 'C2', 0)).toBeNull()
  })
})
```

- [ ] **Step 2: Run the test to verify it fails**

`cd client/ui && pnpm vitest run src/lib/selectors.test.ts`. Expected: `nextTypingExpiry is not a function`.

- [ ] **Step 3: Write the minimal implementation**

`client/ui/src/lib/selectors.ts`, append:

```ts
/** When the next typing indicator in `channelId` runs out (ms timestamp), or null if nobody is typing. */
export function nextTypingExpiry(s: AppState, channelId: string, now: number): number | null {
  const live = Object.values(s.typing[channelId] ?? {}).filter((until) => until > now)
  return live.length ? Math.min(...live) : null
}
```

`client/ui/src/components/TextChannel.svelte`:
- Line 11: add `nextTypingExpiry` to the selectors import (after F4: `import { displayName, nextTypingExpiry,
  stableList, typingNames } from '../lib/selectors'`).
- Replace

  ```ts
    // fade typing indicators on time, not just on events
    $effect(() => {
      const t = setInterval(() => (now = Date.now()), 1000)
      return () => clearInterval(t)
    })
  ```

  with

  ```ts
    // Fade typing indicators on time, not just on events: one timer for the next expiry, and only
    // while someone is typing (an always-on 1 s interval woke the app all day).
    $effect(() => {
      const next = nextTypingExpiry(app.state, channelId, now)
      if (next === null) return
      const t = setTimeout(() => (now = Date.now()), Math.max(0, next - Date.now()) + 50)
      return () => clearTimeout(t)
    })
  ```

The delay uses `Date.now()`, not `now`, because `now` can be minutes old once nobody has typed for a while. An expiry
that has already passed fires at once and corrects itself.

- [ ] **Step 4: Run the tests to verify they pass**

`cd client/ui && pnpm test && pnpm check`: green.

Manual check: a second profile types once. "X is typing…" disappears about 6 s later. With nobody typing, the DevTools
Performance "Timer fired" events from `TextChannel` are 0 over 60 s (before: 60).

- [ ] **Step 5: Commit**

```bash
git add client/ui/src/lib/selectors.ts client/ui/src/lib/selectors.test.ts client/ui/src/components/TextChannel.svelte
git commit -m "perf(ui): typing indicator wakes once per expiry, not every second"
```

---

### Task 27 (F6): Voice levels go to the UI only while a meter is on screen

**Why:** opt-client.md M1. The levels loop (`voice/mod.rs:674-684`) emits `VoiceEvent::Levels` every 100 ms for the
whole call. Each emit is a JSON serialisation plus a WebView2 `ExecuteScript` (tauri's event emit evaluates JS), and
then a Svelte flush. Only the voice settings meter reads them (`VoiceSettings.svelte:16,101`). Expected: about 36k
fewer webview evals per hour of call, and fewer renderer and browser-process wakeups, roughly 0.5–2% of a core.

**Files:**
- Modify: `client/src-tauri/src/voice/controls.rs` (struct lines 5–11; impl after `playout_on`, line 40; tests)
- Modify: `client/src-tauri/src/commands.rs` `restore_session` (lines 185–189): reset the meter count on every page load
- Modify: `client/src-tauri/src/voice/mod.rs` (after `pub type EventSink`, line 264; levels loop lines 674–684;
  `VoiceManager` after `controls()`, line 862; tests before `mic_is_published_at_64_kbps_with_dtx_and_red`, line 981)
- Modify: `client/src-tauri/src/commands.rs` (new command before `/// Hotkey path`, line 468)
- Modify: `client/src-tauri/src/lib.rs` (`invoke_handler`, after `commands::stop_mic_test,`)
- Modify: `client/ui/src/lib/voice.svelte.ts` (`voiceApi`, after `stopMicTest`, line 92)
- Modify: `client/ui/src/components/VoiceSettings.svelte` (before `onMount(async …`, line 25)
- Test: `client/src-tauri/src/voice/controls.rs`, `client/src-tauri/src/voice/mod.rs`, Create
  `client/ui/src/components/VoiceSettings.test.ts`

**Interfaces:**
- Consumes: `Shared::{take_levels, gate_open, controls}`.
- Produces:
  - `Controls::{watch_meters(bool), reset_meters(), meters_shown()}` (F11 uses the counter too).
  - `VoiceManager::{watch_meters(bool), reset_meters()}`. `restore_session` calls `reset_meters()`, because it runs
    once per page load (`App.svelte` boot).
  - The Tauri command `watch_voice_meters { on: bool }` and the UI's `voiceApi.watchMeters(on)`.
  - The mic test is unchanged: it still sends levels, because it only runs while the settings page is open.

- [ ] **Step 1: Write the failing tests**

`client/src-tauri/src/voice/controls.rs`, at the top of `mod tests` (after `use super::*;`):

```rust
    #[test]
    fn meters_are_shown_while_any_view_watches() {
        let mut c = Controls::default();
        assert!(!c.meters_shown());
        c.watch_meters(true);
        c.watch_meters(true); // a second view (or a reopen racing the close)
        c.watch_meters(false);
        assert!(c.meters_shown(), "one view still open");
        c.watch_meters(false);
        c.watch_meters(false); // an unmatched close can't go negative
        assert!(!c.meters_shown());
        c.watch_meters(true);
        assert!(c.meters_shown(), "reopening after a stray close still shows levels");
    }

    #[test]
    fn a_reloaded_page_forgets_the_old_pages_meter_views() {
        let mut c = Controls::default();
        c.watch_meters(true); // settings open when the renderer died: its close never comes
        c.reset_meters(); // the new page's restore_session
        assert!(!c.meters_shown());
        c.watch_meters(true);
        c.watch_meters(false);
        assert!(!c.meters_shown(), "the new page's open/close pair balances again");
    }
```

This covers a page reload while the voice settings are open (on Linux the renderer-crash handler reloads the page).
Without the reset, the old view's +1 never gets its close, and levels flow for the rest of the session.

`client/src-tauri/src/voice/mod.rs`, in `mod tests` before `fn mic_is_published_at_64_kbps_with_dtx_and_red`:

```rust
    #[test]
    fn levels_go_to_the_ui_only_while_a_meter_is_shown() {
        let controls = Arc::new(Mutex::new(Controls::default()));
        let mixer = Arc::new(Mutex::new(Mixer::new(INTERNAL_RATE, PEER_BUFFER_MS)));
        let shared = Shared::new(&AudioConfig::default(), controls.clone(), mixer);
        shared.add_mic_level(0.3, true);
        assert!(meter_update(&shared).is_none(), "nobody is looking");
        controls.lock().unwrap().watch_meters(true);
        shared.add_mic_level(0.3, true);
        match meter_update(&shared) {
            Some(VoiceEvent::Levels { mic, gate_open, .. }) => {
                assert!((mic - 0.3).abs() < 1e-3, "only this tick's level, got {mic}");
                assert!(gate_open);
            }
            other => panic!("expected levels, got {other:?}"),
        }
        controls.lock().unwrap().watch_meters(false);
        shared.add_mic_level(0.3, true);
        assert!(meter_update(&shared).is_none(), "meter closed");
    }
```

`client/ui/src/components/VoiceSettings.test.ts` (this needs F3's `resolve.conditions` in `vite.config.ts`):

```ts
// @vitest-environment jsdom
import { flushSync, mount, unmount } from 'svelte'
import { beforeEach, describe, expect, it, vi } from 'vitest'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: unknown) => invoke(cmd, args) }))

import VoiceSettings from './VoiceSettings.svelte'

const meterCalls = () => invoke.mock.calls.filter(([cmd]) => cmd === 'watch_voice_meters').map(([, a]) => a)

describe('VoiceSettings meter subscription', () => {
  beforeEach(() => invoke.mockClear())

  it('asks for voice levels while open and stops when closed', () => {
    const c = mount(VoiceSettings, { target: document.body })
    flushSync()
    expect(meterCalls()).toEqual([{ on: true }])
    unmount(c)
    expect(meterCalls()).toEqual([{ on: true }, { on: false }])
  })

  it('every open is paired with one close, so reopening keeps the meter alive', () => {
    for (let i = 0; i < 3; i++) {
      const c = mount(VoiceSettings, { target: document.body })
      flushSync()
      unmount(c)
    }
    expect(meterCalls()).toEqual([{ on: true }, { on: false }, { on: true }, { on: false }, { on: true }, { on: false }])
  })
})
```

- [ ] **Step 2: Run the tests to verify they fail**

- `cargo test -p pulse-client --lib -- voice::controls voice::tests::levels`: compile errors (`no method named
  watch_meters`, `no method named reset_meters`, `cannot find function meter_update`).
- `cd client/ui && pnpm vitest run src/components/VoiceSettings.test.ts`: 2 failed (`expected [] to deeply equal [ {
  on: true } ]`).

- [ ] **Step 3: Write the minimal implementation**

`client/src-tauri/src/voice/controls.rs`. Add a field after `muted_before_deafen: bool,`:

```rust
    /// How many UI views show the voice meter (the voice settings page). Lives here because, like
    /// mute, it outlasts a session and the audio thread reads it. Not part of the UI's state.
    #[serde(skip)]
    meter_views: u16,
```

and add after `fn playout_on`:

```rust
    /// A view showing the voice meter appeared (`true`) or went away (`false`).
    pub fn watch_meters(&mut self, on: bool) {
        self.meter_views = if on {
            self.meter_views.saturating_add(1)
        } else {
            self.meter_views.saturating_sub(1)
        };
    }

    /// The page (re)loaded: views from the old page can't send their close any more.
    pub fn reset_meters(&mut self) {
        self.meter_views = 0;
    }

    /// Levels go to the UI only while this holds.
    pub fn meters_shown(&self) -> bool {
        self.meter_views > 0
    }
```

`client/src-tauri/src/voice/mod.rs`, after `pub type EventSink = …;`:

```rust
/// This tick's meter update, if a meter is on screen (each one is a JSON eval in the webview: 10 a
/// second for a whole call otherwise). The levels are taken either way, so a meter that opens
/// mid-call starts from the last 100 ms, not an average over the call so far.
fn meter_update(shared: &Shared) -> Option<VoiceEvent> {
    let (mic, speaker) = shared.take_levels();
    let shown = shared.controls.lock().unwrap().meters_shown();
    shown.then(|| VoiceEvent::Levels {
        mic,
        speaker,
        gate_open: shared.gate_open.load(Ordering::Relaxed),
    })
}
```

In the levels task, replace

```rust
                    let (mic, speaker) = shared.take_levels();
                    let queued = shared.mixer.lock().unwrap().max_buffered();
                    shared
                        .stats
                        .sample_recv_queue((queued * 1000 / INTERNAL_RATE as usize) as u32);
                    let gate_open = shared.gate_open.load(Ordering::Relaxed);
                    events(VoiceEvent::Levels {
                        mic,
                        speaker,
                        gate_open,
                    });
```

with

```rust
                    let queued = shared.mixer.lock().unwrap().max_buffered();
                    shared
                        .stats
                        .sample_recv_queue((queued * 1000 / INTERNAL_RATE as usize) as u32);
                    if let Some(levels) = meter_update(&shared) {
                        events(levels);
                    }
```

In `impl VoiceManager`, after `pub fn controls(&self)`:

```rust
    /// A voice meter appeared in (`true`) or left (`false`) the UI. Survives sessions, like mute.
    pub fn watch_meters(&self, on: bool) {
        self.controls.lock().unwrap().watch_meters(on);
    }

    /// A fresh page: meters shown by the previous one are gone.
    pub fn reset_meters(&self) {
        self.controls.lock().unwrap().reset_meters();
    }
```

`client/src-tauri/src/commands.rs` `restore_session`: add the `voice` state and reset first. Replace

```rust
pub async fn restore_session(
    app: AppHandle,
    core: State<'_, Core>,
) -> Result<Option<User>, ApiError> {
    crate::startup::mark("restore_begin");
```

with

```rust
pub async fn restore_session(
    app: AppHandle,
    core: State<'_, Core>,
    voice: State<'_, VoiceManager>,
) -> Result<Option<User>, ApiError> {
    crate::startup::mark("restore_begin");
    // Runs once per page load: a reload (renderer crash) can't leave the old page's meters counted.
    voice.reset_meters();
```

`VoiceManager` is already imported in `commands.rs` (line 15). The UI call is unchanged: Tauri injects `State` and it
is not an argument.

`client/src-tauri/src/commands.rs`, before `/// Hotkey path (IPC on Linux, global shortcut on Windows)…`:

```rust
/// A voice meter appeared in (`on`) or left the UI: in a call, levels are sent only while one shows.
#[tauri::command]
pub fn watch_voice_meters(voice: State<'_, VoiceManager>, on: bool) {
    voice.watch_meters(on);
}
```

`client/src-tauri/src/lib.rs`, in `generate_handler!`: add `commands::watch_voice_meters,` after
`commands::stop_mic_test,`. App commands are not ACL-gated: `capabilities/default.json` only lists plugin
permissions and `build.rs` has no command manifest.

`client/ui/src/lib/voice.svelte.ts`, in `voiceApi` after `stopMicTest: …,`:

```ts
  /** Levels are only sent while something shows them: call with true when a meter appears, false when it goes. */
  watchMeters: (on: boolean) => invoke('watch_voice_meters', { on }),
```

`client/ui/src/components/VoiceSettings.svelte`, before `onMount(async () => {`:

```ts
  // The in-call meter: the core only sends levels while this page is open.
  onMount(() => {
    void voiceApi.watchMeters(true).catch(() => {})
    return () => void voiceApi.watchMeters(false).catch(() => {})
  })
```

The counter, rather than a bool, makes the result independent of the order the two invokes arrive in (Tauri runs
commands concurrently). A close racing a reopen still ends at 1.

- [ ] **Step 4: Run the tests to verify they pass**

- `cargo test -p pulse-client --lib -- voice`: all pass.
- `cd client/ui && pnpm test && pnpm check`: all pass.
- `cargo clippy --workspace --all-targets -- -D warnings`: clean.

Manual check: in a call with settings closed, `pidstat -w -u -p <WebKitWebProcess pid> 1` for 60 s (Windows: Process
Explorer, context-switch delta on the `msedgewebview2.exe` renderer) shows fewer wakeups than before. Opening voice
settings mid-call shows a live meter at once, and so does reopening it.

- [ ] **Step 5: Commit**

```bash
git add client/src-tauri/src/voice/controls.rs client/src-tauri/src/voice/mod.rs client/src-tauri/src/commands.rs client/src-tauri/src/lib.rs client/ui/src/lib/voice.svelte.ts client/ui/src/components/VoiceSettings.svelte client/ui/src/components/VoiceSettings.test.ts
git commit -m "perf(voice): send meter levels to the UI only while a meter is shown"
```

---

### Task 28 (F11): Skip the denoiser while muted (no unmute burst)

**Why:** spec C16. While muted, `processor.rs:380-383` still runs the full denoiser on every 10 ms block, then zeroes
the block. Strong (DeepFilterNet) is most of a call's DSP CPU. Muted time is common (push-to-talk-style use, AFK in a
channel). Expected: denoiser CPU drops to about 0 while muted. APM keeps running, so echo cancellation stays
converged.

The risk is the unmute burst. A denoiser has look-back state (DeepFilterNet: df_order + lookahead = 7 hops; RNNoise:
1). Left idle, that state would still hold audio from just before the mute, and it would come out in the first
unmuted block. The fix keeps the last 8 raw blocks (80 ms) while idle, and replays them into the denoiser (output
discarded) right before the first live block.

The meter: while a meter is shown, the full chain runs even when muted. So the settings meter reads exactly as today.
This depends on F6's `meter_views`.

**Files:**
- Modify: `client/src-tauri/src/voice/controls.rs` (new `denoise_needed`; test)
- Modify: `client/src-tauri/src/voice/processor.rs`:
  - new `PRIME_BLOCKS` and `MicChain` fields (lines 61–67);
  - `MicChain::new` (lines 70–77);
  - new `MicChain::idle`, plus the wake-up at the top of `process` (line 97);
  - `run`'s per-block section (lines 380–383);
  - tests, plus `use std::{cell::RefCell, rc::Rc}` in `mod tests`.
- Test: `client/src-tauri/src/voice/processor.rs`, `client/src-tauri/src/voice/controls.rs`

**Interfaces:**
- Consumes: `Controls::{mic_open, meters_shown}` (F6) and `Denoiser::process`.
- Produces: `Controls::denoise_needed()` and `MicChain::idle(&mut [f32]) -> BlockOut`. `MicChain::process` keeps its
  signature.

- [ ] **Step 1: Write the failing tests**

`client/src-tauri/src/voice/controls.rs` `mod tests`:

```rust
    #[test]
    fn denoiser_idles_only_when_muted_and_unwatched() {
        let mut c = Controls::default();
        assert!(c.denoise_needed(), "talking");
        c.toggle_mute();
        assert!(!c.denoise_needed(), "muted, nobody looking");
        c.watch_meters(true);
        assert!(c.denoise_needed(), "muted but the meter is open");
        c.watch_meters(false);
        c.toggle_deafen();
        assert!(!c.denoise_needed(), "deafened (implies muted)");
    }
```

`client/src-tauri/src/voice/processor.rs` `mod tests`. Add `use std::cell::RefCell;` and `use std::rc::Rc;` next to
the existing `use std::time::{Duration, Instant};`, then insert before `fn meter_level_is_after_cleaning_and_gain`:

```rust
    /// Records the first sample of every block it is given and of every block it hands back.
    /// With `delay`, it returns the previous block, like a real denoiser's look-back buffers.
    struct Spy {
        seen: Rc<RefCell<Vec<f32>>>,
        emitted: Rc<RefCell<Vec<f32>>>,
        delay: Option<Vec<f32>>,
    }
    impl Denoiser for Spy {
        fn process(&mut self, block: &mut [f32]) -> Option<f32> {
            self.seen.borrow_mut().push(block[0]);
            if let Some(prev) = self.delay.as_mut() {
                let input = block.to_vec();
                block.copy_from_slice(prev);
                *prev = input;
            }
            self.emitted.borrow_mut().push(block[0]);
            Some(1.0)
        }
    }
    type Log = Rc<RefCell<Vec<f32>>>;
    fn spy(delay: bool) -> (Box<Spy>, Log, Log) {
        let (seen, emitted) = (Log::default(), Log::default());
        let d = Spy {
            seen: seen.clone(),
            emitted: emitted.clone(),
            delay: delay.then(|| vec![0.0; FRAME]),
        };
        (Box::new(d), seen, emitted)
    }

    #[test]
    fn idle_skips_the_denoiser_and_sends_silence() {
        let (d, seen, _) = spy(false);
        let mut c = MicChain::new(NsLevel::Off, false, 0.0, 1.0);
        c.set_denoiser(NsLevel::Standard, d);
        for k in 0..20 {
            let mut b = vec![0.5f32; FRAME];
            let out = c.idle(&mut b);
            assert!(
                b.iter().all(|v| *v == 0.0) && !out.sent && out.level == 0.0,
                "block {k} leaked while idle"
            );
        }
        assert!(seen.borrow().is_empty(), "the denoiser ran while idle");
    }

    #[test]
    fn waking_replays_the_last_80ms_once_before_the_live_block() {
        let (d, seen, _) = spy(false);
        let mut c = MicChain::new(NsLevel::Off, false, 0.0, 1.0);
        c.set_denoiser(NsLevel::Standard, d);
        for k in 0..20 {
            let mut b = vec![k as f32 / 100.0; FRAME];
            c.idle(&mut b);
        }
        let mut live = vec![0.99f32; FRAME];
        c.process(&mut live, true, Instant::now());
        let want: Vec<f32> = (12..20).map(|k| k as f32 / 100.0).chain([0.99]).collect();
        assert_eq!(*seen.borrow(), want);
        let mut next = vec![0.5f32; FRAME];
        c.process(&mut next, true, Instant::now());
        assert_eq!(seen.borrow().len(), want.len() + 1, "replayed only once");
    }

    #[test]
    fn unmute_does_not_leak_audio_from_before_the_mute() {
        let (d, _, emitted) = spy(true);
        let mut c = MicChain::new(NsLevel::Off, false, 0.0, 1.0);
        c.set_denoiser(NsLevel::Standard, d);
        let t0 = Instant::now();
        for k in 0..5 {
            let mut b = vec![0.5f32; FRAME]; // talking
            c.process(&mut b, true, t0 + Duration::from_millis(k * 10));
        }
        for _ in 0..50 {
            let mut b = vec![0.01f32; FRAME]; // muted, quiet room
            c.idle(&mut b);
        }
        let mut b = vec![0.01f32; FRAME];
        c.process(&mut b, true, t0 + Duration::from_millis(600));
        let first_live = *emitted.borrow().last().unwrap();
        assert_eq!(first_live, 0.01, "the denoiser emitted pre-mute speech on unmute");
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

`cargo test -p pulse-client --lib -- voice::controls voice::processor`. Expected: compile errors (`no method named
denoise_needed`, `no method named idle`).

I checked that the burst test catches the bug: with an `idle` that only clears `recent` on wake (no replay), it fails
with `left: 0.5, right: 0.01`.

- [ ] **Step 3: Write the minimal implementation**

`client/src-tauri/src/voice/controls.rs`, after `meters_shown` (F6):

```rust
    /// The mic chain has to clean audio: it goes out, or the meter shows the cleaned level.
    /// Muted with no meter on screen, the denoiser can idle.
    pub fn denoise_needed(&self) -> bool {
        self.mic_open() || self.meters_shown()
    }
```

`client/src-tauri/src/voice/processor.rs`. Replace the `MicChain` struct with

```rust
/// Blocks kept while the denoiser idles (muted, no meter open), replayed into it on waking so its
/// look-back holds the last 80 ms of real audio, not what came just before the mute (that would
/// leak out as a blip on unmute). DeepFilterNet looks back df_order + lookahead = 7 hops; 8 covers it.
const PRIME_BLOCKS: usize = 8;

/// One block's journey after APM: denoise → gain (soft-limited) → gate → mute.
pub struct MicChain {
    level: NsLevel,
    denoiser: Box<dyn Denoiser>,
    gate: VoiceGate,
    gain: f32,
    /// The latest blocks seen while idle, oldest first (at most PRIME_BLOCKS).
    recent: VecDeque<Vec<f32>>,
    idle: bool,
}
```

In `MicChain::new`, after `gain,` add:

```rust
            recent: VecDeque::with_capacity(PRIME_BLOCKS),
            idle: false,
```

Replace the head of `process`:

```rust
    pub fn process(&mut self, block: &mut [f32], mic_open: bool, now: Instant) -> BlockOut {
        let prob = self.denoiser.process(block);
```

with

```rust
    /// Muted with no meter on screen: send silence (as a muted `process` does) without running the
    /// denoiser, and remember the block for waking up.
    pub fn idle(&mut self, block: &mut [f32]) -> BlockOut {
        let mut kept = if self.recent.len() == PRIME_BLOCKS {
            self.recent.pop_front().unwrap_or_default()
        } else {
            Vec::with_capacity(block.len())
        };
        kept.clear();
        kept.extend_from_slice(block);
        self.recent.push_back(kept);
        self.idle = true;
        self.gate.reset();
        block.fill(0.0);
        BlockOut {
            level: 0.0,
            sent: false,
        }
    }

    pub fn process(&mut self, block: &mut [f32], mic_open: bool, now: Instant) -> BlockOut {
        if self.idle {
            // Back from idle: bring the denoiser's state up to date (output discarded) first.
            self.idle = false;
            for mut b in std::mem::take(&mut self.recent) {
                self.denoiser.process(&mut b);
            }
        }
        let prob = self.denoiser.process(block);
```

In `run`, replace

```rust
            let mic_open = shared.controls.lock().unwrap().mic_open();
            let t = Instant::now();
```

with

```rust
            let (mic_open, denoise) = {
                let c = shared.controls.lock().unwrap();
                (c.mic_open(), c.denoise_needed())
            };
            if !denoise {
                // Muted, no meter on screen: skip the denoiser (Strong is most of a call's CPU).
                // APM above keeps running, so echo cancellation is still converged on unmute.
                let out = chain.idle(&mut block);
                shared.add_mic_level(out.level, out.sent);
                let chunk: Vec<i16> = block.iter().map(|v| to_i16(*v)).collect();
                if mic_tx.send((INTERNAL_RATE, chunk)).is_err() {
                    return; // session gone
                }
                continue;
            }
            let t = Instant::now();
```

What stays the same:
- The mic test builds its own `Controls::default()` (never muted), so it always runs the full chain.
- Idle blocks aren't counted by the overload detector. The one-off replay on wake (8 blocks) counts as a single
  sample, and a 3 s window mean can't trip on one sample (see `one_spike_does_not_downgrade`).

- [ ] **Step 4: Run the tests to verify they pass**

- `cargo test -p pulse-client --lib -- voice`: all pass.
- `cargo clippy --workspace --all-targets -- -D warnings`: clean.
- `just voice-it`: green.

Manual check on Strong: mute for 30 s, say "test" just before unmuting, then unmute and stay quiet. The other client
hears no blip of "test". `top -H -p <pulse-app pid>` shows the `pulse-strong` thread near 0% while muted.

- [ ] **Step 5: Commit**

```bash
git add client/src-tauri/src/voice/controls.rs client/src-tauri/src/voice/processor.rs
git commit -m "perf(voice): skip the denoiser while muted; replay 80 ms on unmute so nothing stale leaks"
```

---

### Task 29 (F2): WebView2 drops to its Low memory target when minimised or in the background

**Why:** opt-client.md H4. Pulse runs minimised or in the background all day, and WebView2 holds about 200 MB of the
roughly 310 MB in-call PSS. Microsoft's guidance is to set `MemoryUsageTargetLevel = Low` on inactive webviews. A Tauri
2.12 app measured private memory going from 121.9 MB to 36.3 MB. Expected: 60–90 MB less while backgrounded. Never
combine this with `TrySuspend`, because JS must keep handling `pulse://event` (message sounds, unread counts).

**Files:**
- Create: `client/src-tauri/src/webview_memory.rs`
- Modify: `client/src-tauri/src/lib.rs` (module list lines 1–15; `create_main_window` lines 42–43)
- Modify: `client/src-tauri/Cargo.toml` (`[target.'cfg(windows)'.dependencies]`, last section)
- Test: `client/src-tauri/src/webview_memory.rs` (`#[cfg(test)] mod tests`; the policy is plain logic, so it runs on Linux)

**Interfaces:**
- Consumes: `tauri::WebviewWindow::{on_window_event, with_webview}`, `PlatformWebview::controller()` (Windows),
  `webview2_com::…::ICoreWebView2_19::SetMemoryUsageTargetLevel`.
- Produces: `webview_memory::{MemPolicy, Action, Target, BACKGROUND_AFTER}`, `webview_memory::install(&WebviewWindow)`
  (Windows only).

- [ ] **Step 1: Write the failing test**

Create `client/src-tauri/src/webview_memory.rs` with only the tests for now:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minimising_goes_low_at_once_and_focus_restores_normal() {
        let mut p = MemPolicy::default();
        assert_eq!(p.minimised(), Action::Set(Target::Low));
        assert_eq!(p.minimised(), Action::Nothing, "already low");
        assert_eq!(p.unfocused(), Action::Nothing, "already low");
        assert_eq!(p.focused(), Action::Set(Target::Normal));
        assert_eq!(p.focused(), Action::Nothing, "already normal");
    }

    #[test]
    fn the_background_goes_low_only_when_the_timer_fires() {
        let mut p = MemPolicy::default();
        let Action::Arm(g) = p.unfocused() else {
            panic!("expected a timer")
        };
        assert_eq!(p.timer_fired(g), Action::Set(Target::Low));
        assert_eq!(p.focused(), Action::Set(Target::Normal));
    }

    #[test]
    fn coming_back_before_the_timer_cancels_it() {
        let mut p = MemPolicy::default();
        let Action::Arm(g) = p.unfocused() else {
            panic!("expected a timer")
        };
        assert_eq!(p.focused(), Action::Nothing);
        assert_eq!(p.timer_fired(g), Action::Nothing, "stale timer");
        let Action::Arm(g2) = p.unfocused() else {
            panic!("expected a timer")
        };
        assert_eq!(p.timer_fired(g), Action::Nothing, "the first timer stays stale");
        assert_eq!(p.timer_fired(g2), Action::Set(Target::Low));
    }

    #[test]
    fn minimising_after_losing_focus_does_not_wait_for_the_timer() {
        let mut p = MemPolicy::default();
        let Action::Arm(g) = p.unfocused() else {
            panic!("expected a timer")
        };
        assert_eq!(p.minimised(), Action::Set(Target::Low));
        assert_eq!(p.timer_fired(g), Action::Nothing);
    }
}
```

In `client/src-tauri/src/lib.rs`, add `pub mod webview_memory;` after `pub mod voice;` (line 15).

- [ ] **Step 2: Run the test to verify it fails**

`cargo test -p pulse-client --lib -- webview_memory`. Expected: it doesn't compile (`cannot find type MemPolicy`).

- [ ] **Step 3: Write the minimal implementation**

Put this above the tests in `client/src-tauri/src/webview_memory.rs`:

```rust
//! WebView2 memory: drop to the Low memory target while Pulse is minimised or has sat in the
//! background a while, and back to Normal on focus. This is Microsoft's advice for inactive
//! webviews. It is never combined with TrySuspend, which would stop the page handling events
//! (message sounds, unread counts). The policy is plain logic; only applying it is Windows-only.

use std::time::Duration;

/// Unfocused (but not minimised) this long counts as "in the background".
pub const BACKGROUND_AFTER: Duration = Duration::from_secs(120);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Normal,
    Low,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    Nothing,
    /// Apply this target now.
    Set(Target),
    /// Call `timer_fired(generation)` after `BACKGROUND_AFTER`.
    Arm(u64),
}

/// Which target the webview should have. Every focus change starts a new generation, so a timer
/// armed before the user came back can't lower memory under them.
#[derive(Debug, Default)]
pub struct MemPolicy {
    generation: u64,
    low: bool,
}

impl MemPolicy {
    pub fn focused(&mut self) -> Action {
        self.generation += 1;
        if !self.low {
            return Action::Nothing;
        }
        self.low = false;
        Action::Set(Target::Normal)
    }

    pub fn unfocused(&mut self) -> Action {
        self.generation += 1;
        if self.low {
            Action::Nothing
        } else {
            Action::Arm(self.generation)
        }
    }

    pub fn minimised(&mut self) -> Action {
        self.generation += 1;
        self.go_low()
    }

    pub fn timer_fired(&mut self, generation: u64) -> Action {
        if generation == self.generation {
            self.go_low()
        } else {
            Action::Nothing
        }
    }

    fn go_low(&mut self) -> Action {
        if self.low {
            return Action::Nothing;
        }
        self.low = true;
        Action::Set(Target::Low)
    }
}

/// Follow the main window's focus and minimise state.
#[cfg(windows)]
pub fn install(win: &tauri::WebviewWindow) {
    use std::sync::{Arc, Mutex};
    let policy = Arc::new(Mutex::new(MemPolicy::default()));
    let w = win.clone();
    win.on_window_event(move |e| {
        let action = match e {
            tauri::WindowEvent::Focused(true) => policy.lock().unwrap().focused(),
            tauri::WindowEvent::Focused(false) => policy.lock().unwrap().unfocused(),
            // Windows reports a minimised window as resized to 0 x 0.
            tauri::WindowEvent::Resized(size) if size.width == 0 && size.height == 0 => {
                policy.lock().unwrap().minimised()
            }
            _ => return,
        };
        apply(&w, &policy, action);
    });
}

#[cfg(windows)]
fn apply(
    win: &tauri::WebviewWindow,
    policy: &std::sync::Arc<std::sync::Mutex<MemPolicy>>,
    action: Action,
) {
    match action {
        Action::Nothing => {}
        Action::Set(target) => set_target(win, target),
        Action::Arm(generation) => {
            let (win, policy) = (win.clone(), policy.clone());
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(BACKGROUND_AFTER).await;
                let next = policy.lock().unwrap().timer_fired(generation);
                apply(&win, &policy, next);
            });
        }
    }
}

#[cfg(windows)]
fn set_target(win: &tauri::WebviewWindow, target: Target) {
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW, COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL,
        ICoreWebView2_19,
    };
    use windows_core::Interface;
    let level = match target {
        Target::Low => COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW,
        Target::Normal => COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL,
    };
    let sent = win.with_webview(move |wv| {
        // SAFETY: plain COM calls on the webview's own thread (with_webview runs us there).
        let set = unsafe {
            wv.controller()
                .CoreWebView2()
                .and_then(|core| core.cast::<ICoreWebView2_19>())
                .and_then(|v19| v19.SetMemoryUsageTargetLevel(level))
        };
        match set {
            Ok(()) => tracing::info!(?target, "webview memory target"),
            // ICoreWebView2_19 needs WebView2 Runtime 114+; older runtimes just stay on Normal.
            Err(e) => tracing::warn!(?target, error = %e, "webview memory target not set"),
        }
    });
    if let Err(e) = sent {
        tracing::warn!(error = %e, "webview memory target: webview unavailable");
    }
}
```

In `client/src-tauri/src/lib.rs` `create_main_window`, replace

```rust
    #[cfg(not(target_os = "linux"))]
    let _ = win;
```

with

```rust
    // WebView2: the Low memory target while minimised or in the background.
    #[cfg(windows)]
    webview_memory::install(&win);
    #[cfg(not(any(target_os = "linux", windows)))]
    let _ = win;
```

In `client/src-tauri/Cargo.toml`, extend the Windows section:

```toml
[target.'cfg(windows)'.dependencies]
tauri-plugin-global-shortcut = "2"
# WebView2 memory target (webview_memory.rs). Keep these on the versions tauri/wry use (see
# Cargo.lock), or the COM types don't match (E0308).
webview2-com = "0.39"
windows-core = "0.62"
```

Cargo.lock already has webview2-com 0.39.1 and windows-core 0.62.2, so no new versions come in. The detection uses the
`Resized(0×0)` event and not `is_minimized()`, so the handler never calls back into the window from its own event
callback.

- [ ] **Step 4: Run the tests to verify they pass**

- `cargo test -p pulse-client --lib -- webview_memory`: 4 passed.
- `XWIN_ACCEPT_LICENSE=1 cargo xwin clippy -p pulse-client --target x86_64-pc-windows-msvc -- -D warnings`: Finished,
  no warnings (this is what type-checks the `cfg(windows)` code).
- `cargo clippy --workspace --all-targets -- -D warnings`: clean on Linux.

Windows measurement (part of the owner's one Windows test). In PowerShell, sum the private bytes of `pulse-app.exe` and its
whole WebView2 process tree:

```powershell
$root = (Get-Process pulse-app | Select-Object -First 1).Id
$all = Get-CimInstance Win32_Process
$tree = @($root)
do { $n = $tree.Count; $tree += $all | Where-Object { $tree -contains $_.ParentProcessId -and $tree -notcontains $_.ProcessId } | ForEach-Object ProcessId } while ($tree.Count -ne $n)
'{0:N1} MB' -f (($tree | ForEach-Object { (Get-Process -Id $_).PrivateMemorySize64 } | Measure-Object -Sum).Sum / 1MB)
```

Run it focused, then after 60 s minimised, then 10 s after restoring. Expected:
- Minimised: at least 60 MB less than focused.
- `pulse.log` has `webview memory target target=Low`, then `target=Normal` on restore.
- After restoring, clicking a channel responds without a visible hitch.

- [ ] **Step 5: Commit**

```bash
git add client/src-tauri/src/webview_memory.rs client/src-tauri/src/lib.rs client/src-tauri/Cargo.toml Cargo.lock
git commit -m "feat(client): WebView2 low memory target while minimised or in the background"
```

---

### Task 30 (F7): Measure libwebrtc's log volume in a call; filter at the source only if it is chatty

**Why:** opt-client.md M2. `webrtc-sys-0.3.47/src/webrtc.cpp:163` registers the Rust log sink at `LS_VERBOSE`, so no
`RTC_LOG` is ever a no-op. Every line is formatted in C++ and copied across cxx into a `String`, and then our filter
(`info,livekit=warn,libwebrtc=warn`, `logging.rs:21-22`) drops it. The volume is unknown: today's log files have 0
`libwebrtc` lines, because they are filtered. The research rule is to measure first, and below 5 lines/s to drop the
item. Possible gain: 1–3% of a core in a call.

**Files:**
- Create: `client/src-tauri/tests/webrtc_log_volume.rs`
- Modify: `client/src-tauri/Cargo.toml` (`[dev-dependencies]`: `log = "0.4"`, already in Cargo.lock at 0.4.34)
- Modify: `justfile` (new recipe after `voice-it`)
- Only if Step 2 fails: Create `vendor/webrtc-sys/` (copy of the crate), Modify `Cargo.toml` (`[patch.crates-io]`,
  `exclude`)

**Interfaces:** Consumes `VoiceManager::join` with `AudioMode::Null`, `pulse_server::testing`, and the dev LiveKit.
Produces a number (lines/s) and, if needed, a patched `webrtc-sys`.

- [ ] **Step 1: Write the measurement test**

`client/src-tauri/tests/webrtc_log_volume.rs`:

```rust
//! How chatty is libwebrtc in a call? webrtc-sys registers its log sink at LS_VERBOSE, so every
//! RTC_LOG line is formatted in C++ and handed to Rust, where our filter (libwebrtc=warn) drops it.
//! This counts those lines. Needs the dev LiveKit and .env. Run: `just webrtc-log-volume`.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use pulse_client::api::Api;
use pulse_client::voice::devices::AudioConfig;
use pulse_client::voice::{AudioMode, VoiceManager};
use pulse_server::testing;

/// Above this, filtering at the source (a patched webrtc-sys) is worth its upkeep.
const CHATTY_PER_SEC: f64 = 5.0;

static LINES: AtomicU64 = AtomicU64::new(0);

struct CountLibwebrtc;

impl log::Log for CountLibwebrtc {
    fn enabled(&self, m: &log::Metadata) -> bool {
        m.target() == "libwebrtc"
    }
    fn log(&self, r: &log::Record) {
        if r.target() == "libwebrtc" {
            LINES.fetch_add(1, Ordering::Relaxed);
        }
    }
    fn flush(&self) {}
}

static LOGGER: CountLibwebrtc = CountLibwebrtc;

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn libwebrtc_log_lines_per_second_in_a_call() {
    log::set_logger(&LOGGER).expect("logger");
    log::set_max_level(log::LevelFilter::Trace);

    // The dev LiveKit from .env (`cp .env.example .env`); `just` loads it (dotenv-load).
    let mut cfg = testing::test_config();
    cfg.livekit_url = std::env::var("PULSE_LIVEKIT_URL").expect("PULSE_LIVEKIT_URL: run via just");
    cfg.livekit_secret =
        std::env::var("PULSE_LIVEKIT_SECRET").expect("PULSE_LIVEKIT_SECRET: run via just");
    let app = testing::spawn_with(cfg).await;
    let (_, alex) = testing::register(&app, "alex").await;
    let s = testing::create_server(&app, &alex, "Main").await;
    let (_, sam) = testing::register(&app, "sam").await;
    let r = testing::post_json(
        &app,
        &sam,
        &format!("/servers/{}/join", s.id),
        serde_json::json!({}),
    )
    .await;
    assert!(r.status().is_success(), "join server: {}", r.status());
    let api = Api::new(&format!("http://{}", app.addr));
    let lounge = api
        .channels(&alex, s.id)
        .await
        .unwrap()
        .into_iter()
        .find(|c| c.name.as_deref() == Some("Lounge"))
        .unwrap();

    let a = VoiceManager::new(std::sync::Arc::new(|_| {}));
    let b = VoiceManager::new(std::sync::Arc::new(|_| {}));
    for (vm, token) in [(&a, &alex), (&b, &sam)] {
        vm.join(
            &api,
            token,
            lounge.id,
            AudioConfig::default(),
            AudioMode::Null(48_000),
        )
        .await
        .unwrap();
    }
    tokio::time::sleep(Duration::from_secs(5)).await; // connect chatter is not the steady state
    let start = LINES.load(Ordering::Relaxed);
    let secs = 60;
    tokio::time::sleep(Duration::from_secs(secs)).await;
    let per_sec = (LINES.load(Ordering::Relaxed) - start) as f64 / secs as f64;
    println!("libwebrtc: {per_sec:.1} log lines/s in a two-person call");
    a.leave().await;
    b.leave().await;
    assert!(
        per_sec < CHATTY_PER_SEC,
        "{per_sec:.1} lines/s: filter libwebrtc's logging at the source"
    );
}
```

`justfile`, after the `voice-it` recipe (just loads `.env`, which holds the dev LiveKit URL and secret):

```just
# libwebrtc log lines per second in a two-person call (decides whether to filter at the source)
webrtc-log-volume: dev-livekit
    cargo test -p pulse-client --test webrtc_log_volume -- --ignored --nocapture
```

`client/src-tauri/Cargo.toml` `[dev-dependencies]`, after `tempfile = "3"`:

```toml
# tests/webrtc_log_volume.rs counts libwebrtc's log lines (it logs through the `log` facade)
log = "0.4"
```

- [ ] **Step 2: Run it (this is the decision)**

`just webrtc-log-volume` (needs `.env`: `cp .env.example .env` once). Write down the printed `lines/s`.

- **Passes (< 5 lines/s):** the logging is not worth a patch. Skip Steps 3–4 and commit the test with the number in
  the message.
- **Fails (≥ 5 lines/s):** go to Step 3. Real devices add ADM lines, so on Windows optionally also run the owner's call with
  `PULSE_LOG=info,libwebrtc=trace` and `grep -c libwebrtc pulse.log*` over 120 s.

- [ ] **Step 3: Write the implementation (only if Step 2 failed)**

```bash
mkdir -p vendor
cp -r ~/.cargo/registry/src/index.crates.io-*/webrtc-sys-0.3.47 vendor/webrtc-sys
sed -i 's/AddLogToStream(this, webrtc::LoggingSeverity::LS_VERBOSE)/AddLogToStream(this, webrtc::LoggingSeverity::LS_WARNING)/' vendor/webrtc-sys/src/webrtc.cpp
grep -n 'AddLogToStream' vendor/webrtc-sys/src/webrtc.cpp   # expect LS_WARNING
```

Root `Cargo.toml`. Change `exclude = ["spikes"]` to `exclude = ["spikes", "vendor/webrtc-sys"]`, so it doesn't become
a workspace member that clippy and tests would check. Then append:

```toml
# libwebrtc formats every RTC_LOG line at LS_VERBOSE and hands it to Rust only to be dropped
# (measured: <N> lines/s in a call, tests/webrtc_log_volume.rs). The vendored copy registers its
# sink at LS_WARNING; the only change is src/webrtc.cpp's AddLogToStream. Drop this when upstream
# makes the severity follow log::max_level().
[patch.crates-io]
webrtc-sys = { path = "vendor/webrtc-sys" }
```

The crate is about 6.8 MB of source. The prebuilt libwebrtc is still downloaded by its build script, as before.

- [ ] **Step 4: Run it again (only if Step 3 was done)**

Run the Step 2 command again. Expected: pass, well under 5 lines/s (only warnings and errors cross now). Then:
- `XWIN_ACCEPT_LICENSE=1 cargo xwin check -p pulse-client --target x86_64-pc-windows-msvc`: Finished.
- `cargo test --workspace`: green.

- [ ] **Step 5: Commit**

```bash
# measurement only:
git add client/src-tauri/tests/webrtc_log_volume.rs client/src-tauri/Cargo.toml Cargo.lock justfile
git commit -m "test(voice): measure libwebrtc log volume in a call (<N> lines/s, below the 5/s bar)"
# or, with the patch:
git add client/src-tauri/tests/webrtc_log_volume.rs client/src-tauri/Cargo.toml Cargo.toml Cargo.lock justfile vendor/webrtc-sys
git commit -m "perf(voice): register libwebrtc's log sink at LS_WARNING (<N> lines/s formatted and dropped before)"
```

---

### Task 31 (F8): Bundle only the latin-script font subsets, as woff2

**Why:** opt-client.md M4. `main.ts:3-6` imports `@fontsource/onest/{400,500,600,700}.css`. That is 6 subsets × 4
weights × woff2 + woff: 48 files, 582,704 B in the binary, though WebView2 only ever fetches what it needs. Spec C16
says "latin-only (−500 KB)". This plan keeps **latin + latin-ext + symbols** (woff2 only, 12 files, measured
145,676 B), which saves **437,028 B**. Why keep the extra two subsets:
- Accented names (latin-ext) would otherwise change font.
- The keybind labels ← → (U+2190/2192, served from the symbols subset today) would otherwise change font.

Unicode ranges are copied verbatim from fontsource, so every glyph that rendered in Onest still comes from the same
face. Cyrillic and Vietnamese message text falls back to system-ui. That is the one visible difference, and only for
those scripts.

**Files:**
- Create: `client/ui/src/fonts.css` (generated once by the script in Step 3)
- Modify: `client/ui/src/main.ts` (lines 3–6)
- Create: `client/ui/src/fonts.test.ts`

**Interfaces:** Consumes the files in `@fontsource/onest/files/*.woff2` and its 400.css ranges. Produces the same
`font-family: 'Onest'` that `app.css:19` uses.

- [ ] **Step 1: Write the failing test**

`client/ui/src/fonts.test.ts`:

```ts
/// <reference types="node" />
// Bundled fonts: latin scripts only, woff2 only, and for those glyphs the same faces as before.
import { readFileSync } from 'node:fs'
import { createRequire } from 'node:module'
import { describe, expect, it } from 'vitest'

const read = (rel: string) => readFileSync(new URL(rel, import.meta.url), 'utf8')
const upstream = readFileSync(createRequire(import.meta.url).resolve('@fontsource/onest/400.css'), 'utf8')

const faces = (css: string) =>
  [...css.matchAll(/@font-face\s*{([^}]*)}/g)].map(([, body]) => ({
    weight: /font-weight:\s*(\d+)/.exec(body)?.[1],
    urls: [...body.matchAll(/url\(['"]?([^'")]+)/g)].map((m) => m[1]),
    range: /unicode-range:\s*([^;]+);/.exec(body)?.[1].trim(),
  }))
const subsetOf = (url: string) => /onest-(.+)-\d{3}-normal/.exec(url)?.[1]

describe('bundled fonts', () => {
  it('main.ts loads fonts.css instead of the full @fontsource sets', () => {
    const main = read('./main.ts')
    expect(main).toContain("import './fonts.css'")
    expect(main).not.toMatch(/@fontsource\/onest\/\d+\.css/)
  })

  it('ships only latin, latin-ext and symbols, as woff2, for the four weights the UI uses', () => {
    const ours = faces(read('./fonts.css'))
    expect(ours).toHaveLength(12)
    for (const f of ours) {
      expect(f.urls).toHaveLength(1)
      expect(f.urls[0]).toMatch(/\.woff2$/)
      expect(['latin', 'latin-ext', 'symbols']).toContain(subsetOf(f.urls[0]))
    }
    expect(new Set(ours.map((f) => f.weight))).toEqual(new Set(['400', '500', '600', '700']))
  })

  it('uses the same unicode ranges as @fontsource, so those glyphs come from the same face', () => {
    const ranges = new Map(faces(upstream).map((f) => [subsetOf(f.urls[0]), f.range]))
    for (const f of faces(read('./fonts.css'))) expect(f.range).toBe(ranges.get(subsetOf(f.urls[0])))
  })
})
```

Baseline measurement: `cd client/ui && pnpm build && ls dist/assets/*.woff* | wc -l && du -cb dist/assets/*.woff* |
tail -1`. Expected: `48` and `582704`.

- [ ] **Step 2: Run the test to verify it fails**

`cd client/ui && pnpm vitest run src/fonts.test.ts`. Expected: 3 failed. `main.ts` doesn't import `./fonts.css`, and
`ENOENT … fonts.css`.

- [ ] **Step 3: Write the minimal implementation**

Generate `client/ui/src/fonts.css` once from fontsource's own ranges, and commit the output. The output is
deterministic: 12 `@font-face` blocks, 9,887 B.

```bash
cd client/ui && node --input-type=module <<'EOF'
// One-off: write src/fonts.css from @fontsource/onest's own ranges (see the header comment).
import { readFileSync, writeFileSync } from 'node:fs'
const upstream = readFileSync('node_modules/@fontsource/onest/400.css', 'utf8')
const range = (subset) =>
  new RegExp(`onest-${subset}-400-normal \\*/[^}]*unicode-range: ([^;]+);`).exec(upstream)[1]
const head = `/* Onest, bundled locally (the app makes no third-party requests). Only the subsets the UI and
   latin-script names use, woff2 only (WebView2 and WebKitGTK both read it): the full set was 48
   files / 583 KB. Ranges are copied from @fontsource/onest so the same glyphs come from the same
   face; other scripts (Cyrillic, Vietnamese) fall back to system-ui. */`
const faces = [400, 500, 600, 700].flatMap((w) =>
  ['symbols', 'latin-ext', 'latin'].map(
    (subset) => `@font-face {
  font-family: 'Onest';
  font-style: normal;
  font-display: swap;
  font-weight: ${w};
  src: url('@fontsource/onest/files/onest-${subset}-${w}-normal.woff2') format('woff2');
  unicode-range: ${range(subset)};
}`,
  ),
)
writeFileSync('src/fonts.css', [head, ...faces].join('\n') + '\n')
EOF
```

The order symbols → latin-ext → latin per weight matches fontsource's declaration order, so overlapping ranges resolve
the same way (the later face wins). The first face it writes, for example:

```css
@font-face {
  font-family: 'Onest';
  font-style: normal;
  font-display: swap;
  font-weight: 400;
  src: url('@fontsource/onest/files/onest-latin-400-normal.woff2') format('woff2');
  unicode-range: U+0000-00FF,U+0131,U+0152-0153,U+02BB-02BC,U+02C6,U+02DA,U+02DC,U+0304,U+0308,U+0329,U+2000-206F,U+20AC,U+2122,U+2191,U+2193,U+2212,U+2215,U+FEFF,U+FFFD;
}
```

Vite resolves the bare `@fontsource/...` specifier inside `url()` (I checked this with `vite build`).

`client/ui/src/main.ts`, replace lines 3–6

```ts
import '@fontsource/onest/400.css'
import '@fontsource/onest/500.css'
import '@fontsource/onest/600.css'
import '@fontsource/onest/700.css'
```

with

```ts
import './fonts.css'
```

- [ ] **Step 4: Run the tests to verify they pass**

- `cd client/ui && pnpm test && pnpm check`: green.
- `pnpm build && ls dist/assets/*.woff* | wc -l && du -cb dist/assets/*.woff* | tail -1`: `12` and `145676`.

Visual check in `just dev-client`: the UI looks identical (names, headings at 600/700, the ← → keybind labels).

- [ ] **Step 5: Commit**

```bash
git add client/ui/src/fonts.css client/ui/src/fonts.test.ts client/ui/src/main.ts
git commit -m "perf(ui): bundle only latin-script Onest subsets as woff2 (583 -> 146 KB)"
```

---

### Task 32 (F9): Ship the UI sounds once (in the Rust binary), not again as webview assets

**Why:** opt-client.md L3. `client/ui/public/sounds/*.wav` (7 files, 47,928 B) are copied into `dist/` and embedded as
webview assets. They are also `include_bytes!`'d by `sounds.rs:13-19`, which is the only player (cpal; WebKitGTK must
not play audio). Nothing under `ui/src` references `/sounds/`. Saving: 47,928 B.

**Files:**
- Move: `client/ui/public/sounds/` → `client/src-tauri/sounds/` (`public/` is then empty and goes too)
- Modify: `client/src-tauri/src/sounds.rs` (lines 13–19 paths; new test after `every_bundled_sound_decodes`)
- Modify: `scripts/gen_sounds.py` (line 6 `OUT`)

**Interfaces:** None change. `sounds::load(name)` returns the same data.

- [ ] **Step 1: Write the failing test**

`client/src-tauri/src/sounds.rs`, in `mod tests` after `every_bundled_sound_decodes`:

```rust
    #[test]
    fn sounds_ship_once_in_the_binary_not_again_as_webview_assets() {
        // Played by the core (cpal), never by the page: a copy under ui/public would be bundled twice.
        let public = concat!(env!("CARGO_MANIFEST_DIR"), "/../ui/public/sounds");
        assert!(!std::path::Path::new(public).exists(), "{public} is bundled into the webview assets too");
    }
```

- [ ] **Step 2: Run the test to verify it fails**

`cargo test -p pulse-client --lib -- sounds`. Expected: `sounds_ship_once…` FAILED (`…/ui/public/sounds is bundled
into the webview assets too`).

- [ ] **Step 3: Write the minimal implementation**

```bash
git mv client/ui/public/sounds client/src-tauri/sounds
sed -i 's#include_bytes!("../../ui/public/sounds/#include_bytes!("../sounds/#' client/src-tauri/src/sounds.rs
sed -i 's#^OUT = "client/ui/public/sounds"#OUT = "client/src-tauri/sounds"#' scripts/gen_sounds.py
```

After this, `sounds.rs` reads, for example, `"join" => include_bytes!("../sounds/join.wav"),` for all seven names.

- [ ] **Step 4: Run the tests to verify they pass**

- `cargo test -p pulse-client --lib -- sounds`: 3 passed (`every_bundled_sound_decodes` still decodes all seven).
- `cd client/ui && pnpm build && test ! -e dist/sounds && echo ok`: `ok`.

- [ ] **Step 5: Commit**

```bash
git add -A client/ui/public client/src-tauri/sounds client/src-tauri/src/sounds.rs scripts/gen_sounds.py
git commit -m "build(client): ship UI sounds once, in the core (not again as webview assets)"
```

---

### Task 33 (F12): One duplicate crate fewer: tokio-tungstenite 0.27 → 0.29

**Why:** opt-client.md L1. On the client's Windows graph, `cargo tree -d` shows `tokio-tungstenite`/`tungstenite` at
0.27 (ours, `client/src-tauri/Cargo.toml:27`) and at 0.29 (via `livekit-net`). Bumping ours removes one copy, about
100–200 KB. The API changes between 0.27 and 0.29 don't touch what we use: `connect_async`, `accept_async`,
`Message::text`, `CloseCode`, `CloseFrame`, with no matches on `Error::Http`. The server's dev-dependency (tests only)
moves too, so Cargo.lock has a single version.

The others in that list need upstream upgrades: rubato 0.14 (deep_filter), prost 0.11, rand 0.8, itertools (tract
0.21.4, pinned), and windows 0.54 (cpal 0.15).

**Files:**
- Modify: `client/src-tauri/Cargo.toml` (line 27)
- Modify: `server/Cargo.toml` (`[dev-dependencies]`, line 43)
- Test: the measurement command below; the existing gateway tests (client `gateway.rs` tests, `server/tests/*.rs`)

**Interfaces:** None change.

- [ ] **Step 1: Write the measurement (the "test")**

```bash
cargo tree -p pulse-client -e normal -d --target x86_64-pc-windows-msvc | grep -E '^(tokio-)?tungstenite v'
```

Expected after the change: no output.

- [ ] **Step 2: Run it to verify it "fails"**

Expected now:

```
tokio-tungstenite v0.27.0
tokio-tungstenite v0.29.0
tungstenite v0.27.0
tungstenite v0.29.0
```

- [ ] **Step 3: Write the minimal implementation**

`client/src-tauri/Cargo.toml`:

```toml
tokio-tungstenite = { version = "0.29", features = ["rustls-tls-native-roots"] }
```

`server/Cargo.toml` `[dev-dependencies]`:

```toml
tokio-tungstenite = "0.29"
```

Cargo.lock already contains 0.29.0, so the build resolves offline.

- [ ] **Step 4: Run the measurement and the tests**

- Step 1's command: no output.
- `grep -c 'name = "tokio-tungstenite"' Cargo.lock`: `1`.
- `cargo test -p pulse-client --lib -- gateway`, then `cargo test -p pulse-server`: green.
- `XWIN_ACCEPT_LICENSE=1 cargo xwin check -p pulse-client --target x86_64-pc-windows-msvc`: Finished.

- [ ] **Step 5: Commit**

```bash
git add client/src-tauri/Cargo.toml server/Cargo.toml Cargo.lock
git commit -m "build: tokio-tungstenite 0.29 (one copy, shared with livekit)"
```

---

### Task 34 (F1): Release profile (fat LTO, one codegen unit, strip)

**Why:** opt-client.md H3. There is no `[profile.release]`, so release builds use Cargo's defaults: 16 codegen units,
no LTO, and on Linux the symbols stay in (about 20 MB). The Windows exe is 67,150,336 B (about 22.5 MB as LZMA). The
Linux binary is 94,418,856 B (main checkout, Oct 15). **Measured with this profile: Linux 60,968,208 B (−33.4 MB,
−35%)**, built from scratch in 5 min 37 s on 32 cores. Expected on Windows: −3 to −6 MB of exe and −1 to −2 MB of
installer.

**Files:**
- Modify: `Cargo.toml` (insert after `[workspace.dependencies]`, i.e. after line 21 `pulse-protocol = { path = "protocol" }`)
- Test: the measurement commands below (build config, no unit test)

**Interfaces:** Consumes nothing. Produces smaller release artifacts for `release-windows.yml` (`tauri build` +
`cargo build --release -p pulse-server`) and `deploy/server.Dockerfile` (the server image picks up the same
workspace profile).

- [ ] **Step 1: Write the measurement (the "test")**

```bash
# From the repo root. Same commands before and after the change.
cargo build --release -p pulse-client --bin pulse-app
stat -c '%n %s' target/release/pulse-app
XWIN_ACCEPT_LICENSE=1 cargo xwin build --release -p pulse-client --bin pulse-app --target x86_64-pc-windows-msvc
stat -c '%n %s' target/x86_64-pc-windows-msvc/release/pulse-app.exe
# Proxy for the NSIS (LZMA) installer payload:
xz -6 -T0 -c target/x86_64-pc-windows-msvc/release/pulse-app.exe | wc -c
```

Expected after the change:
- Linux: about 61,000,000 B, at most 65,000,000 B (before: 94,418,856 B; measured 60,968,208 B while writing this plan).
- Windows exe: at most 64,000,000 B (before: 67,150,336 B).
- xz: below 22,000,000 B (before: about 22.5 MB).

The plain `cargo build` leaves out tauri's `custom-protocol` feature that `tauri build` adds. That makes no measurable
difference to size, and the same command is used before and after.

- [ ] **Step 2: Run it to see the baseline**

Run the block above on the commit before this task. Expected: Linux about 94.4 MB (unstripped: `file
target/release/pulse-app` says "not stripped"); Windows exe about 67.15 MB.

- [ ] **Step 3: Write the minimal implementation**

In `Cargo.toml`, after the `[workspace.dependencies]` table:

```toml
# Release builds (installer, test kit, server image). Fat LTO + one codegen unit: smaller, slightly
# faster code (tract/ndarray generics are otherwise duplicated across 16 units); it costs build
# time only. strip: the Linux binary carried ~20 MB of symbols (Windows writes no PDB). Panics still
# unwind: a panicking tokio task (gateway, outbox, voice) must not take the whole app down.
[profile.release]
lto = "fat"
codegen-units = 1
strip = true
```

What stays the same:
- `opt-level` stays 3, because tract/rustfft are real-time hot paths.
- `panic` is not set to `abort` (see the comment).
- The `[profile.dev.package.*]` overrides for DeepFilterNet/tract only apply to `dev`/`test`, so this doesn't touch
  them.
- Crash reports keep working: the panic hook in `logging.rs` writes `file:line` from `PanicHookInfo::location()`,
  which strip doesn't remove.

- [ ] **Step 4: Run the measurement again, plus the Windows cross-build**

Run the Step 1 block. Expected: the numbers under "Expected after the change", and `file target/release/pulse-app`
says "stripped". Also run:
- `cargo test -p pulse-client --lib`: unchanged (tests use the dev profile).
- `XWIN_ACCEPT_LICENSE=1 cargo xwin check -p pulse-client --target x86_64-pc-windows-msvc`: Finished. The cross-build
  goes through lld-link. Rust's LTO only merges Rust bitcode; libwebrtc and the cxx/C++ objects link as before.

After merging, trigger `release-windows` by hand (`workflow_dispatch`). Note the job time (about 10 min before) and the
`*-setup.exe` size from the artifact list printed by "Assemble test kit".

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml
git commit -m "build: release profile with fat LTO, one codegen unit and strip

Linux pulse-app <before> -> <after> B, Windows exe <before> -> <after> B (xz <before> -> <after>)."
```

---

---

## Wrap-up

### Task 35 (X1): Before/after numbers, the Windows build, and the test protocol

**Files:**
- Create: `docs/superpowers/research/opt-results.md` (the before/after table; the Windows column is filled in after the owner's test)
- Modify: `.github/workflows/release-windows.yml:14` (`LIVEKIT_VERSION: 1.13.7` → `1.13.10`, matching prod since 2026-10-15)
- Test: `just check`, `just voice-it`, local two-client runs

**Interfaces:**
- Consumes: every log field added by earlier tasks:
  - from R4: `ns -X.XdB gate NN%`;
  - from R5: `accel`/`decel`/`silent`, and the verdict in `docs/superpowers/research/opt-neteq-buffer.md`;
  - from L7b/L7a: `adjust`, `mix_busy`;
  - from L3/L4b: `token_prefetched`, `pipeline_reused`;
  - the step-0 lines: join phases, startup phases, `voice: path`.
- Produces: `opt-results.md`, and a `release-windows` run on `optimisation-run` for the owner's test.

**Why:** the spec's "one big dev run, then one big Windows test", measured against the 2026-10-15 baseline (Windows, two instances, Sonar mic):

| Metric | Baseline |
|---|---|
| Startup ready | 2.4 s |
| Join | 2.0 s (`room_connect` 1.6 s) |
| RTT | 160 ms (ISP route via London; not ours to fix) |
| Jitter | ~1 ms |
| NetEq buffer / target | 80–100 / 20–40 ms |
| Receive queue | p50 20 ms |
| Strong ns+gate | p50 0.57 ms / max 1.9 ms per 10 ms |

- [ ] **Step 1: Full local check**

Run: `just check`
Expected: fmt, clippy, all tests, types and UI checks pass. Then `just voice-it`: all ignored real-LiveKit tests pass, including the new ones from R2, L2c, L4a and L4b.

- [ ] **Step 2: Windows cross-check**

Run: `XWIN_ACCEPT_LICENSE=1 cargo xwin clippy -p pulse-client --target x86_64-pc-windows-msvc -- -D warnings`
Expected: no warnings. This covers the MMCSS (R3) and WebView2 (F2) code, which Linux CI never compiles.

- [ ] **Step 3: Local before/after on Linux (same machine, same LiveKit)**

Build `main` (before) and `optimisation-run` (after) in release mode against `just dev-server` + `just dev-livekit`. For each build:
- launch two profiles (`./target/release/pulse-app` and `PULSE_PROFILE=b ./target/release/pulse-app`);
- join, talk for 60 s, switch channel once, then leave;
- collect from each profile's log: `startup phase="ready"`, `voice: joined elapsed_ms`, the four join phases, and the `voice: path` lines (median `buffer`, `target`, `recv` p50, `underrun`).

Repeat the after-run with `sudo tc qdisc add dev lo root netem delay 80ms` (RTT ≈ 160 ms, like the Windows PC's route). Ask the owner before running `sudo`, and remove it afterwards with `sudo tc qdisc del dev lo root`.

Expected:
- join ≈ 1.0–1.2 s at 160 ms RTT, down from ≈ 1.75–2.0 s;
- a channel switch shows `pipeline_reused=true`;
- `recv` p50 ≤ 20 ms with no sustained `underrun`;
- startup `ready` 0.6–1.0 s sooner than before.

Record the numbers in `opt-results.md`.

- [ ] **Step 4: Size**

Run: `ls -l target/release/pulse-app` for both builds, plus the NSIS installer size from the `release-windows` artifact (Step 6).
Expected: the Linux binary ~33 MB smaller (F1 measured 94.4 → 61.0 MB), and fonts −437 KB (F8). Record both.

- [ ] **Step 5: Write `opt-results.md`**

```markdown
# Optimisation run: results

Baseline: Windows 2026-10-15 (two instances, one PC, Sonar mic, ISP RTT 160 ms). Linux numbers: laptop, local LiveKit.

| Metric | Baseline (Windows) | Linux before | Linux after | Windows after |
|---|---|---|---|---|
| Startup → Ready | 2.4 s | | | |
| Join (elapsed_ms) | 2.0 s | | | |
| room_connect_ms | 1.6 s | | | |
| Channel switch | — | | | |
| NetEq buffer / target | 80–100 / 20–40 ms | | | |
| recv queue p50 | 20 ms | | | |
| Strong ns+gate p50/max | 0.57 / 1.9 ms | | | |
| ns attenuation / gate open | — | | | |
| Linux binary / Windows installer | — / — | | | |

NetEq verdict (R5): see opt-neteq-buffer.md.
```

Fill the Linux columns now. The Windows column is filled after the owner's test.

- [ ] **Step 6: Bump the kit's LiveKit, commit, push the branch, build the Windows kit**

Edit `.github/workflows/release-windows.yml` line 14 to `LIVEKIT_VERSION: 1.13.10`.

```bash
git add .github/workflows/release-windows.yml docs/superpowers/research/opt-results.md
git commit -m "docs(optimisation): before/after numbers; Windows kit on LiveKit 1.13.10

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
git push -u origin optimisation-run
gh workflow run release-windows.yml --ref optimisation-run
```

Pushing a personal branch is pre-authorised; main is not touched. Expected: the run succeeds and the `pulse-windows-kit` artifact contains the NSIS installer.

- [ ] **Step 7: Server deploy, owner's OK required**

S1, S3, S4 and L1's Caddyfile only take effect on prod after a deploy, and the Windows test should run against them. Before asking, tell the owner:
- S3 adds migration 0004, so an older image won't boot against the migrated DB. Rolling back means `just restore` of the pre-deploy backup (R2 backups must be configured first).
- `compose.yml` changes (`stop_signal`, `PULSE_LIVEKIT_API_URL`) need `docker compose up -d`, not just a pull.

With the OK:
1. Run `gh workflow run release-server.yml --ref optimisation-run`. It publishes `:<sha7>`. Check first whether a dispatch also moves `:latest`; if it does, say so to the owner, because `latest` would then point at an unmerged branch.
2. Run `just deploy <sha7>`, then `just smoke`.

- [ ] **Step 8: Hand the owner the Windows test protocol** (from the spec)

1. Install the new kit. Join voice with both instances, mics open, and talk for 60 s with Strong on.
2. Switch channel once, then switch back.
3. Minimise Pulse for a minute, then restore it.
4. Play something through the headphones (a show) while talking for 60 s. R4's `ns`/`gate` numbers now capture the "Strong clips words while a show plays" effect.
5. Optional: Wi-Fi/Ethernet off for 5 s, then back on, to check R1/R2's recovery.
6. Send a bug report from **each** instance at the end.

Then pull the reports (`just reports`, `just report <id> --log`) and fill the Windows column of `opt-results.md`.

---

## Appendix: drafters' notes (risks, dependencies, covered failure modes)

### Server


**Cross-task dependencies**
- S3 and S4 use `crate::db::begin_write` from S2; S3's code edits the transaction block S2 creates in `messages::send`. Run S2 before S3/S4.
- S4's `server_boots_and_serves_when_livekit_is_down` uses S1's `spawn_binary`; S1's harness already sets `PULSE_LIVEKIT_API_URL` (harmless before S4).
- S1 and S4 both edit `server/src/lib.rs::serve` and `deploy/compose.yml` (different lines); apply S1 first.
- S4 touches `server/src/testing.rs` (`test_config`): any other section that constructs `Config` literally must add `livekit_api_url`.
- Migration number `0004` (S3) must stay unique if another section also adds a migration (e.g. the index nits in opt-server.md L2); renumber in landing order.

**Risks**
- **Rollback after S3:** sqlx refuses to boot an older image against a DB that has a migration it doesn't know (`VersionMissing`). Rolling back past S3 means restoring the pre-deploy backup that `just deploy` takes, or first shipping `Migrator::set_ignore_missing(true)`. Say so in the deploy notes.
- **`sqlx::migrate!` and new files:** the macro is expanded at compile time; if only a `.sql` file changed, `touch server/src/db.rs` (S3 also edits Rust code in the crate, so a normal build picks it up).
- **`synchronous=NORMAL`:** an OS crash or power loss can lose the last commits (never corrupts). Bounded by the nightly backups; accepted in the spec.
- **S1 compose `stop_signal`** applies only to containers created from the new compose file, so the first deploy after this lands still waits ~10 s; the SIGTERM handler in the image fixes it from then on regardless.
- **S2 concurrency tests are races:** they fail most runs before the fix, not every run. The PRAGMA test is the deterministic red step.
- **S4 new dependency surface:** the `services` feature brings in `reqwest` (no TLS features) and `livekit-region`. A `wss://` API URL fails cleanly (fallback path), which is why compose sets a loopback `http://` URL.
- **S4 mute/deafen flags after a restart:** `declared` flags are in memory, so restored members show unmuted until their client sends `VoiceState` again. The client doesn't re-announce flags on a gateway reconnect today (`client/src-tauri/src/commands.rs:375-403` only on toggle/join). That needs a client-side follow-up (re-send `ClientFrame::VoiceState` after Ready while in voice); it belongs to the client section.
- **S4 small ghost window:** someone who leaves between `fetch` and `apply` (milliseconds) is restored and stays until their next join/leave. Acceptable; the background retry path can widen this to one request.

**Failure modes from the first draft, now covered by steps**
1. **LiveKit accepts TCP but hangs at boot.** Covered: S4 Step 1, `hanging_livekit_at_boot_is_bounded_and_falls_back` (boot finishes in under 3 s, falls back, and closes the old rows).
2. **LiveKit down at boot, back later (host reboot ordering).** Covered: S4 Step 1, `livekit_up_late_is_picked_up_by_the_retry_loop`, using the `retry_loop(s, started_at, every, tries)` parameterisation added in S4 Step 3. Small residual risk: the test reserves a port, releases it and binds it again, so another process could take it in between (unlikely on CI).
3. **Outbox retry after the user deleted the message.** Covered: S3 Step 1, `retry_after_delete_returns_the_deleted_original`; no extra implementation needed. Still unchecked: how the UI renders a pending item confirmed by a deleted message. That belongs to the client section.

### Client robustness


**Dependencies and merge order (shared files):**
- **R3 (cpal 0.18) goes before anything else that touches `voice/devices.rs`**, especially the playout section (output callback / `on_output` / `Playout`) and B9 (keeping the pipeline across a channel switch). After R3, any new code must use `devices::device_name()`, `cpal::Error`, `u32` sample rates and configs passed by value. A section written against 0.15 won't compile.
- **R4 and the gate section (spec B11 / opt-voice H1: the gate decides on the undelayed signal)** both edit `MicChain::process` (`processor.rs:97-110`). Whichever lands second keeps the three `BlockOut` fields: `ns_in` measured before the denoiser, `ns_out` after it (before gain), and `judged = mic_open`.
- **R2 and B9 (channel switch closes the old room in the background)** both rewrite `join`/`leave_locked` in `voice/mod.rs`. B9 must keep these: `join()` bumps `epoch` and sets `last_join`; `leave()` bumps `epoch` and clears it; a rejoin calls `join_inner(.., Some(epoch))`, and that does not announce `Disconnected` for the dead session.
- **R1 and B10 (gateway starts in `setup` with the stored token)** both touch `commands.rs::activate` / `lib.rs` setup. B10 should start the gateway through `Core::activate`, which after R6 uses `spawn_full` with the flags closure. R1's `.on_window_event` uses `try_state`, so it is safe before `Core` exists. If the footprint section (C13, WebView2 memory target on minimise/blur) also adds `.on_window_event`, merge both into one closure for clarity. Tauri allows several listeners, but one place is easier to read.
- **R5 changes `netstats::summarize` to take `&Totals`** (call site `voice/mod.rs:459-460`). Any section adding path-line fields must rebase onto the new `PathStats`/`Summary` `Display` strings (the exact-string tests in `netstats.rs` and `stats.rs` will tell).
- R3 changes what the baseline means: cpal 0.18 WASAPI timestamps include hardware latency, so `device ms in/out` will read higher on Windows. Default configs also now prefer 48 kHz, so fewer resamples. Say so when comparing against the 2026-10-15 baseline.
- R5 Step 6 needs `sudo tc` (the owner's go-ahead). Step 7 runs only on the "artifact (DTX)" verdict. The note's `Verdict:` line is what the playout task reads.
- R3's MMCSS can only be verified on Windows (log lines in Step 5). There is no Windows CI test job, and `release-windows` builds but doesn't run tests.

**Risks:**
- cpal 0.18 is a large jump (0.15 → 0.18: unified errors, new I24/U24 defaults, timestamp semantics). The Linux behaviour is pinned by `voice_it real_devices_are_not_reopened_without_a_reason` and the ignored ALSA naming test. Windows naming relies on `description().name()` preferring `FriendlyName` (verified in `cpal-0.18.2/src/host/wasapi/device.rs:412-440`), but it isn't exercised until the owner's test.
- cpal 0.18.2 WASAPI reports a system default-device change on default streams as a `StreamInvalidated` error (`stream.rs:731-736`) while the stream keeps running. Our handler only logs it (throttled) and the 2 s follower reopens as before. Expect one new warning per switch in Windows logs.
- R1's 15 s heartbeat doubles heartbeat traffic (about 20 B per beat per client, trivial). A link with RTT > 1.7 s gets a 3×RTT ack wait, so it isn't falsely declared dead.
- R2 holds the user's session token in memory for the rejoin. That's no change from `Core`, and `deactivate` → `leave()` clears it on logout.

**Failure modes found while planning (each now has its own step):**
1. **Rejoin while a device reopen is in flight (Bluetooth headset):** R2 Step 4b. It adds `Reopener::settle`, `AudioIo::close`, a real close for a reopen whose session is gone, and a waited close on a rejoin's teardown. It also adds two tests.
2. **Resume from sleep with Wi-Fi not yet back, so the backoff has grown:** R1 Step 4b (`a_probe_cuts_a_grown_backoff_short`). No implementation change: R1's probe already breaks the backoff wait.
3. **A 24-bit integer device format (cpal 0.18 ranks I24 above I16):** R3 Step 4b (`twenty_four_bit_samples_round_trip`, I24 and U24). It is coverage only, with a manual-conversion fallback if it fails.

**Remaining gap (no automated test):** MMCSS registration itself, which is Windows only. It is checked by log lines in R3 Step 5.

**R6 notes:** R6 sends the flags after **every** Ready, not only while in voice. That is a deliberate superset, explained in the task's Why. B10 (the gateway started in `setup`) must create the gateway through `Core::activate` or pass the same `AfterReady` closure, or restarts will forget mute again.

### Latency


**Ordering and dependencies.**
- Execution order: L1 → L5 → L3 → L6 → L7b → L8 → L7a → L2a → L2b → L2c → L4a → L4b. Client-robustness R2 (voice
  auto-rejoin) lands before L2a; L4a/L4b carry a Coordination line on what of R2 to preserve. L1, L5, L3, L6, L7b are
  independent of each other. L8 must precede L7a (both edit `Shared`/`mictest.rs`), L2a and L4 (the mic sink is how
  a room is attached and detached). L7a precedes L4b (`mix_in.clear()`). L2a-L2c precede L4b (`side_by_side`,
  `keep_runtime`, the background publish). L3's `prefetch` gets `warm_runtime` in L2b.
- Collisions with other sections: (1) the stats task adding `ns_atten_db`/`gate_open_pct` edits `processor.rs`
  and `stats.rs` like L6/L8: land it first or rebase on L6's `BlockOut.in_level` (see L6 "Coordination");
  L7b/L7a log `adjust`/`mix_busy` as separate tracing fields so `stats.rs`'s `Summary` format is not touched.
  (2) The cpal upgrade touches `devices.rs` (`open_half`, `build_*`, `prefer_internal_rate`); L2a only changes
  `AudioIo::start`'s orchestration and L7a only `Shared`/`on_output`, but all three are in one file: land the cpal
  upgrade before L2a/L7a if possible. (3) "Skip the denoiser while muted" (spec C16) edits `MicChain::process`, as
  L6 does: apply it after L6 and keep `in_level` measured before any early return. (4) "Levels only while shown"
  (spec C15) edits the session levels task that L4b slims down: do C15 after L4b. (5) If another section raises
  `pulse-mic` to real-time priority, note that after L8 that thread calls into WebRTC's capture (a C++ mutex shared
  with the encoder thread): promote both or neither.
- L1's Caddyfile change only takes effect after a deploy, which needs the owner's OK (prod).

**Risks.**
- L6 relies on the engines' real delays (RNNoise 10 ms, DFN 30 ms); `engine_delays_match_what_the_gate_assumes`
  guards it. Standard and Off now decide (and learn the floor) on the raw input level, so in a noisy room a manual
  threshold behaves a little differently than before; auto mode is unaffected in practice (prob gate). A
  Standard ↔ Strong switch mid-word resizes the gate's delay line (one dropped or inserted 10 ms block).
- L7b's 10 ms lowest-fill target suits 10 ms WASAPI periods; devices with big callbacks (Linux ALSA, Bluetooth)
  will underrun once and then refill to 20 ms + their pull size; watch `underrun` per window on Linux before
  calling it done. If it keeps underrunning, make `target` adaptive (raise 5 ms per window with an underrun, decay
  after 10 s clean).
- L7a: peer add/remove allocate or free a little memory inside the speaker callback (HashMap insert, ring drop);
  rare (join/leave), but not strictly allocation-free.
- L5: the crash-report notice (`pulse://auto-report-sent`) can now be emitted before the UI listens, since
  `activate` runs in `setup`; it's cosmetic (the report still goes out). If it matters, gate it on `ui_attached`
  like `forward`.

**Failure modes users might hit: now covered by steps in the owning tasks.**
1. *Hover prefetch, then the session changes* → L3 Steps 6-10 (`a_cleared_cache_gives_nothing`,
   `signing_out_forgets_prefetched_tokens`; `forget_tokens` in `Core::deactivate`). Residual: a token prefetched
   just before the user was removed from the server stays usable for ≤ `FRESH_FOR` (2 min); a server-side kick is
   the real guard.
2. *Switch while the old room is reconnecting, or a switch that fails* → L4a Steps 6-10
   (`a_detached_room_gets_no_more_frames`, `a_failed_switch_ends_disconnected_and_the_old_room_stays_left`;
   `detach_locked` drops the session before closing the room), carried into L4b's `detach_locked`.
3. *Startup with a network that comes up late* → L5 Steps 6-10 (`startup_survives_a_late_network`,
   `failures_within_the_grace_keep_waiting_then_give_up`; `Boot` tolerates pre-Ready failures for `BOOT_GRACE`
   = 10 s).

### Footprint


**Order and dependencies**
- F3 must come before F6: F6's component test needs the `resolve.conditions` added in F3.
- F6 must come before F11: F11 uses `meter_views` / `meters_shown`.
- F4, F5 and F10 all edit `TextChannel.svelte` (imports on lines 10–11 and nearby blocks). Do them in that order and
  re-grep the old strings. F10's import line assumes F4 has landed.
- F1 is independent, but do it last or alone. Fat LTO lengthens every release build, including `release-windows` (about
  10 min today) and the server image on both arches. If CI time hurts, `lto = "thin"` keeps most of the size win.
- F2 also edits `lib.rs`, F6 edits `lib.rs` and `commands.rs`, and other plan sections touch both. Merge by hand, not by
  line number.
- F7 is a decision task. Most likely outcome: only the test is committed. Don't vendor `webrtc-sys` unless Step 2
  fails.

**Risks**
- **F3:** any future in-place write (`app.state.x.push(…)`) would now fail silently, because nothing would re-render.
  The comment in `store.svelte.ts` says so. A grep on this commit found no such writes.
- **F4:** the cache key relies on the server setting `edited_at` on every content change. It does
  (`server/src/messages/routes.rs:255`). Two edits in the same millisecond would show the first. Deleted messages
  never reach `renderMessage`.
- **F8:** Cyrillic and Vietnamese message text renders in system-ui instead of Onest. This is the only visible
  difference in this section, and it is deliberate, per the spec's "latin-only". Keeping latin-ext and symbols makes
  the saving 437 KB instead of the spec's −500 KB.
- **F11:** the replay on wake costs 8 denoiser runs in one block (on Strong, possibly 15–40 ms on a slow PC). The
  processor drops audio older than 60 ms (`MAX_LAG`), so a very slow PC could lose a few ms of the first word after
  unmuting. That matches today's behaviour on overload, and it is recorded in `stats.lag_drops`.
- **F2:** `ICoreWebView2_19` needs WebView2 Runtime 114+. Older runtimes log `webview memory target not set` and stay
  as today. The first click after restoring may page memory back in: Microsoft notes a brief stutter.

**Failure modes a user might hit, now covered by steps above**
1. **Levels never stop after a page reload while the voice settings were open.** Covered in F6:
   `Controls::reset_meters()` is called from `restore_session` on every page load, and tested by
   `a_reloaded_page_forgets_the_old_pages_meter_views`.
2. **Leaving a channel while an older page is loading** (a hole in history, plus a `scrollTo` on a null list).
   Covered in F5: `addOlder` and the null-safe scroll writes, tested by `TextChannel.test.ts` (a component test with a
   deferred `list_messages`).
3. **The unread count or NEW line is wrong after a trim.** Covered in F5: `trimHistory` never cuts at or after the
   first unread message (a channel never read is not trimmed at all), tested by "trimming keeps unread messages".
   Chosen over clamping to the server count because `ReadStateUpdated` carries no count.
