# Bug Reports Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Friends can send a bug report (text + diagnostics + log tail) from the app; panics and unexpected exits are reported automatically on the next launch; Greg reads reports with `just reports`; logs can no longer flood the disk.

**Architecture:** The client gathers everything in Rust (`client/src-tauri/src/reports/`) and POSTs one JSON body to a new authed `POST /reports` route, stored in a `bug_reports` SQLite table. If the server can't be reached, Rust writes a `.txt` to Downloads instead. Crash detection uses marker files in the profile's log dir, turned into a persisted pending report at startup. Greg reads reports through new server CLI subcommands over Tailscale SSH.

**Tech Stack:** Rust (axum 0.8, sqlx/SQLite, reqwest, tracing, tracing-appender), Tauri 2, Svelte 5, vitest; new crate `os_info = "3"`.

**Spec:** `docs/superpowers/specs/2026-10-12-bug-reports-design.md`

## Global Constraints

- Text: 1–4000 characters for `bug` (trimmed, non-empty), 0–4000 for `crash` / `unexpected-exit`.
- meta ≤ 16 KB (serialized JSON), log ≤ 512 KB; route body limit 1 MB; client sends the last **256 KB** of log.
- Rate limit `report_user = Rate::per_hour(5)`.
- Kinds on the wire and in the DB: `bug`, `crash`, `unexpected-exit`.
- Reports are write-only over HTTP: no GET route for them, for anyone.
- Never log or send the token, the password or a request body. Failed-request log lines carry only the URL **path** (no query).
- Repeated audio-stream errors: at most one log line per **10 s** per stream callback. Log file cap: **50 MB** per day per process.
- Saved file name `pulse-report-<YYYYMMDD-HHMMSS>.txt`, folder: Downloads → Desktop → the profile's log dir.
- Pending automatic report dropped when older than **7 days**.
- Copy (exact): modal note "Includes your app version, system info and recent app log."; fail line "Couldn't reach the server."; saved line "Saved to {path} — send it to Greg on WhatsApp."; success "Thanks! Report #{short} sent."; crash notice "Pulse closed unexpectedly last time, so we sent Greg a report."
- Spec refinements decided while planning (record as rulings when executing):
  - **Short id = last 6 characters of the ULID, lookup by suffix**, not prefix: ULID prefixes are timestamps, so reports sent close together share them.
  - **No elapsed time in failed-request lines**: `Api::parse` only sees the response, and threading a timer through ~30 call sites isn't worth it. Path + error is what diagnosis needs.
  - **The 50 MB cap counts bytes written by this process today**, not the file's size on disk (a restart the same day starts the count again). Restarts are rare; a flood is one process.
- Work happens on branch `bug-reports`. Run `just check` (fmt, clippy -D warnings, all tests, ts types, UI check + tests) before the final task's commit.

## Review Focus

1. **The log tail breaks UTF-8 or splits a line**: a 256 KB cut can land mid-character or mid-line; the report must contain valid text starting at a whole line (Task 4 test `tail_starts_at_a_whole_line`).
2. **A panic inside the panic hook or a crash during startup**: writing `crash.json` must never panic (all I/O errors ignored), and a missing log dir must not stop the app (Task 5 test `panic_marker_write_ignores_a_missing_dir`).
3. **A pending report that can never be sent** (server rejects it, e.g. too big or 400): it must not be retried forever — on a 4xx it is dropped, on network/5xx it is kept (Task 5 test `pending_dropped_on_rejection_kept_on_network_error`).
4. **Two profiles on one machine**: markers live in each profile's own log dir, so profile B's clean exit can't hide profile A's crash (Task 5 test uses two dirs: `markers_are_per_dir`).
5. **Concurrent reports and an ambiguous short id**: `report <id>` with a suffix matching two reports must say so rather than show the wrong one (Task 3 test `ambiguous_suffix_is_an_error`).

---

### Task 1: Log hygiene — throttle, size cap, failed-request lines

**Files:**
- Create: `client/src-tauri/src/throttle.rs`
- Modify: `client/src-tauri/src/lib.rs` (add `pub mod throttle;` next to the other `mod` lines)
- Modify: `client/src-tauri/src/logging.rs` (capped writer)
- Modify: `client/src-tauri/src/voice/devices.rs:556,566,580,600` (the four stream-error closures)
- Modify: `client/src-tauri/src/api.rs` (`parse`, `ok_empty`)

**Interfaces:**
- Produces: `throttle::Throttle::new(window: Duration) -> Throttle`, `Throttle::hit(&mut self, now: Instant) -> Option<u64>` (Some(n) = log now, n = repeats suppressed since the last line).
- Produces: `logging::Capped<W: Write>` with `Capped::new(inner: W, cap: u64, day: fn() -> u64)`.
- Produces: `api::path_only(url: &reqwest::Url) -> String`.

- [ ] **Step 1: Write the failing throttle tests** — `client/src-tauri/src/throttle.rs`:

```rust
//! Log-flood guard: an error that repeats (an audio callback failing hundreds of times a second)
//! is logged once, then at most once per window with how many repeats were swallowed.

use std::time::{Duration, Instant};

pub struct Throttle {
    window: Duration,
    last: Option<Instant>,
    suppressed: u64,
}

impl Throttle {
    pub fn new(window: Duration) -> Self {
        Self { window, last: None, suppressed: 0 }
    }

    /// `Some(n)`: log now (n repeats were swallowed since the last line). `None`: stay quiet.
    pub fn hit(&mut self, now: Instant) -> Option<u64> {
        let _ = now;
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_hit_logs() {
        let mut t = Throttle::new(Duration::from_secs(10));
        assert_eq!(t.hit(Instant::now()), Some(0));
    }

    #[test]
    fn repeats_inside_the_window_are_counted_then_reported_once() {
        let start = Instant::now();
        let mut t = Throttle::new(Duration::from_secs(10));
        assert_eq!(t.hit(start), Some(0));
        for i in 1..=500 {
            assert_eq!(t.hit(start + Duration::from_millis(i)), None);
        }
        assert_eq!(t.hit(start + Duration::from_secs(11)), Some(500));
        assert_eq!(t.hit(start + Duration::from_secs(12)), None);
    }
}
```

Add `pub mod throttle;` to `client/src-tauri/src/lib.rs`.

- [ ] **Step 2: Run, expect FAIL**

Run: `cargo test -p pulse-client --lib throttle`
Expected: `first_hit_logs` and `repeats_…` FAIL (`None` vs `Some(0)`).

- [ ] **Step 3: Implement `hit`**

```rust
    pub fn hit(&mut self, now: Instant) -> Option<u64> {
        match self.last {
            Some(at) if now.saturating_duration_since(at) < self.window => {
                self.suppressed += 1;
                None
            }
            _ => {
                self.last = Some(now);
                Some(std::mem::take(&mut self.suppressed))
            }
        }
    }
```

- [ ] **Step 4: Run, expect PASS** — `cargo test -p pulse-client --lib throttle` → 2 passed.

- [ ] **Step 5: Use it in the four cpal error callbacks** (`voice/devices.rs`). Each closure owns its own throttle (cpal error callbacks are `FnMut + Send + 'static`). Replace each `|e| tracing::warn!(error = %e, "input stream error")` with:

```rust
                            {
                                let mut t = crate::throttle::Throttle::new(STREAM_ERROR_EVERY);
                                move |e| {
                                    if let Some(repeated) = t.hit(std::time::Instant::now()) {
                                        tracing::warn!(error = %e, repeated, "input stream error");
                                    }
                                }
                            },
```

(and the same with `"output stream error"` for the two output streams). Add near the top of `devices.rs`:

```rust
/// A broken device can fail every callback; log its error at most this often.
const STREAM_ERROR_EVERY: std::time::Duration = std::time::Duration::from_secs(10);
```

Run: `cargo build -p pulse-client` → builds.

- [ ] **Step 6: Write the failing capped-writer tests** — add to `logging.rs`:

```rust
/// Stops writing after `cap` bytes per UTC day (one notice line marks the cut), so a log flood
/// can't fill the disk. `day` returns the current UTC day number; injectable for tests.
pub struct Capped<W> {
    inner: W,
    cap: u64,
    day: fn() -> u64,
    today: u64,
    written: u64,
}

pub const LOG_CAP_BYTES: u64 = 50 * 1024 * 1024;

pub fn utc_day() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() / 86_400)
        .unwrap_or(0)
}

impl<W: std::io::Write> Capped<W> {
    pub fn new(inner: W, cap: u64, day: fn() -> u64) -> Self {
        Self { inner, cap, day, today: day(), written: 0 }
    }
}

impl<W: std::io::Write> std::io::Write for Capped<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.inner.write(buf)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}
```

and in `mod tests`:

```rust
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};

    static DAY: AtomicU64 = AtomicU64::new(1);
    fn fake_day() -> u64 {
        DAY.load(Ordering::SeqCst)
    }

    #[test]
    fn capped_writer_stops_at_the_cap_with_one_notice_and_resets_next_day() {
        DAY.store(1, Ordering::SeqCst);
        let mut w = Capped::new(Vec::new(), 10, fake_day);
        w.write_all(b"12345\n").unwrap();
        w.write_all(b"67890\n").unwrap(); // crosses the cap: dropped, notice written
        w.write_all(b"more\n").unwrap(); // dropped silently
        let text = String::from_utf8(w.inner.clone()).unwrap();
        assert_eq!(text.matches("log size cap reached").count(), 1, "{text}");
        assert!(text.starts_with("12345\n"));
        assert!(!text.contains("67890") && !text.contains("more"));
        DAY.store(2, Ordering::SeqCst);
        w.write_all(b"fresh\n").unwrap();
        assert!(String::from_utf8(w.inner.clone()).unwrap().ends_with("fresh\n"));
    }
```

- [ ] **Step 7: Run, expect FAIL** — `cargo test -p pulse-client --lib logging` → the new test FAILs (everything is written, no notice).

- [ ] **Step 8: Implement the cap** — replace the `write` body:

```rust
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let day = (self.day)();
        if day != self.today {
            self.today = day;
            self.written = 0;
        }
        if self.written >= self.cap {
            return Ok(buf.len()); // already cut today: swallow
        }
        if self.written + buf.len() as u64 > self.cap {
            self.written = self.cap;
            self.inner
                .write_all(b"WARN pulse: log size cap reached; nothing more is logged today\n")?;
            return Ok(buf.len());
        }
        self.written += buf.len() as u64;
        self.inner.write_all(buf)?;
        Ok(buf.len())
    }
```

Wire it into `init`: `let (writer, guard) = tracing_appender::non_blocking(Capped::new(appender, LOG_CAP_BYTES, utc_day));`

- [ ] **Step 9: Run, expect PASS** — `cargo test -p pulse-client --lib logging` → all pass (including the existing `writes_messages_to_a_file_in_the_dir`).

- [ ] **Step 10: Write the failing path test** — in `api.rs` tests:

```rust
    #[test]
    fn failed_request_lines_keep_only_the_path() {
        let u = reqwest::Url::parse("https://pulsechat.co.za/channels/X/messages?before=Y&token=secret").unwrap();
        assert_eq!(path_only(&u), "/channels/X/messages");
    }
```

with a stub `pub fn path_only(url: &reqwest::Url) -> String { url.to_string() }` above `impl Api`.

- [ ] **Step 11: Run, expect FAIL** — `cargo test -p pulse-client --lib failed_request_lines` → FAIL (query kept).

- [ ] **Step 12: Implement and log failures**

```rust
/// What a failed-request log line may show: the path only (no host, no query, never the token).
pub fn path_only(url: &reqwest::Url) -> String {
    url.path().to_string()
}

fn log_failure(path: Option<String>, what: &str) {
    tracing::warn!(path = path.as_deref().unwrap_or("?"), error = what, "request failed");
}
```

In both `parse` and `ok_empty`, replace the first line with:

```rust
        let r = r.map_err(|e| {
            log_failure(e.url().map(path_only), &e.to_string());
            ApiError::Network(e.to_string())
        })?;
        if !r.status().is_success() {
            log_failure(Some(path_only(r.url())), r.status().as_str());
        }
```

(The rest of each function is unchanged.)

- [ ] **Step 13: Run** — `cargo test -p pulse-client --lib` → all pass. `cargo clippy -p pulse-client --all-targets -- -D warnings` → clean.

- [ ] **Step 14: Commit**

```bash
git add client/src-tauri/src/throttle.rs client/src-tauri/src/lib.rs client/src-tauri/src/logging.rs client/src-tauri/src/voice/devices.rs client/src-tauri/src/api.rs
git commit -m "fix(logs): collapse repeated audio errors, cap each day's log at 50 MB, log failed requests (path only)"
```

---

### Task 2: Server — `bug_reports` table and `POST /reports`

**Files:**
- Create: `server/migrations/0003_bug_reports.sql`
- Create: `server/src/reports/mod.rs`, `server/src/reports/routes.rs`, `server/src/reports/store.rs`
- Modify: `server/src/lib.rs` (`pub mod reports;`, `.merge(reports::routes::router())`)
- Modify: `server/src/limits.rs` (new `report_user` field in `LimitsConfig`, `Default`, `unlimited()`, `Limits`, `Limits::new`)
- Modify: `protocol/Cargo.toml` (`serde_json.workspace = true` under `[dependencies]`), `protocol/src/rest.rs`, `protocol/src/lib.rs` (exports)
- Test: `server/tests/reports.rs`

**Interfaces:**
- Produces (protocol): `rest::ReportKind { Bug, Crash, UnexpectedExit }` (serde `kebab-case`), `rest::CreateReportRequest { kind: ReportKind, text: String, meta: serde_json::Value, log: String }`, `rest::CreateReportResponse { id: String }`.
- Produces (server): `reports::store::insert(db, user: UserId, req: &CreateReportRequest) -> AppResult<String>`, `reports::store::validate(req) -> AppResult<()>`, constants `MAX_TEXT_CHARS = 4000`, `MAX_META_BYTES = 16 * 1024`, `MAX_LOG_BYTES = 512 * 1024`.

- [ ] **Step 1: Protocol types** — append to `protocol/src/rest.rs`:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
pub enum ReportKind {
    Bug,
    Crash,
    UnexpectedExit,
}

impl ReportKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Bug => "bug",
            Self::Crash => "crash",
            Self::UnexpectedExit => "unexpected-exit",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct CreateReportRequest {
    pub kind: ReportKind,
    pub text: String,
    /// App version, OS, connection and voice state, crash details: free-form so new fields need no migration.
    #[ts(type = "Record<string, unknown>")]
    pub meta: serde_json::Value,
    pub log: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct CreateReportResponse {
    pub id: String,
}
```

Add to `protocol/Cargo.toml` `[dependencies]`: `serde_json.workspace = true`. In `export_all` add:

```rust
    rest::ReportKind::export_all(&cfg)?;
    rest::CreateReportRequest::export_all(&cfg)?;
    rest::CreateReportResponse::export_all(&cfg)?;
```

Run: `just gen-types` → new files under `client/ui/src/lib/protocol/`.

- [ ] **Step 2: Write the failing server tests** — `server/tests/reports.rs`:

```rust
//! Bug reports: friends can file them; nobody can read them over HTTP.
mod common;

use common::*;
use pulse_server::limits::{LimitsConfig, Rate};
use serde_json::json;

fn report(kind: &str, text: &str, log: &str) -> serde_json::Value {
    json!({ "kind": kind, "text": text, "meta": { "app_version": "0.1.0" }, "log": log })
}

#[tokio::test]
async fn a_report_is_stored_and_its_id_returned() {
    let app = spawn().await;
    let (alex, token) = register(&app, "alex").await;
    let r = post_json(&app, &token, "/reports", report("bug", "voice cut out", "line1\nline2\n")).await;
    assert_eq!(r.status(), 200);
    let id = r.json::<serde_json::Value>().await.unwrap()["id"].as_str().unwrap().to_string();
    let (user_id, kind, text, log): (String, String, String, String) =
        sqlx::query_as("SELECT user_id, kind, text, log FROM bug_reports WHERE id = ?")
            .bind(&id)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!((user_id, kind.as_str(), text.as_str(), log.as_str()), (alex.to_string(), "bug", "voice cut out", "line1\nline2\n"));
}

#[tokio::test]
async fn bad_reports_are_rejected() {
    let app = spawn().await;
    let (_, token) = register(&app, "alex").await;
    let long = "x".repeat(4001);
    assert_eq!(post_json(&app, &token, "/reports", report("bug", "   ", "")).await.status(), 400);
    assert_eq!(post_json(&app, &token, "/reports", report("bug", &long, "")).await.status(), 400);
    assert_eq!(post_json(&app, &token, "/reports", report("nope", "x", "")).await.status(), 422);
    let big_log = "y".repeat(512 * 1024 + 1);
    assert_eq!(post_json(&app, &token, "/reports", report("bug", "x", &big_log)).await.status(), 400);
    let huge = "z".repeat(1024 * 1024 + 1);
    assert_eq!(post_json(&app, &token, "/reports", report("bug", "x", &huge)).await.status(), 413);
    // automatic reports may be empty
    assert_eq!(post_json(&app, &token, "/reports", report("crash", "", "")).await.status(), 200);
}

#[tokio::test]
async fn reports_need_a_session() {
    let app = spawn().await;
    let r = app.http.post(app.url("/reports")).json(&report("bug", "x", "")).send().await.unwrap();
    assert_eq!(r.status(), 401);
}

#[tokio::test]
async fn five_reports_an_hour() {
    let mut cfg = test_config();
    cfg.limits = LimitsConfig { report_user: Rate::per_hour(5), ..LimitsConfig::unlimited() };
    let app = spawn_with(cfg).await;
    let (_, token) = register(&app, "alex").await;
    for _ in 0..5 {
        assert_eq!(post_json(&app, &token, "/reports", report("bug", "x", "")).await.status(), 200);
    }
    assert_eq!(post_json(&app, &token, "/reports", report("bug", "x", "")).await.status(), 429);
}

#[tokio::test]
async fn reports_cannot_be_read_over_http() {
    let app = spawn().await;
    let (_, token) = register(&app, "alex").await;
    let r = app.http.get(app.url("/reports")).bearer_auth(&token).send().await.unwrap();
    assert!(r.status() == 404 || r.status() == 405, "{}", r.status());
}
```

- [ ] **Step 3: Run, expect FAIL** — `cargo test -p pulse-server --test reports` → compile error: `report_user` missing (LimitsConfig). That is the expected first failure.

- [ ] **Step 4: Limits field** — in `server/src/limits.rs` add to `LimitsConfig`:

```rust
    /// Bug/crash reports per user.
    pub report_user: Rate,
```

`Default`: `report_user: Rate::per_hour(5),` — `unlimited()`: `report_user: r,` — `Limits`: `pub report_user: Limiter,` — `Limits::new`: `report_user: Limiter::new(c.report_user),`.

Run: `cargo test -p pulse-server --test reports` → compiles; tests FAIL (404s: no route, no table).

- [ ] **Step 5: Migration** — `server/migrations/0003_bug_reports.sql`:

```sql
-- Bug, crash and unexpected-exit reports from clients (spec 2026-10-12-bug-reports-design).
CREATE TABLE bug_reports (
    id         TEXT PRIMARY KEY,
    user_id    TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at TEXT NOT NULL,
    kind       TEXT NOT NULL CHECK (kind IN ('bug', 'crash', 'unexpected-exit')),
    text       TEXT NOT NULL,
    meta       TEXT NOT NULL,
    log        TEXT NOT NULL,
    seen_at    TEXT
) STRICT;

CREATE INDEX bug_reports_created ON bug_reports (created_at);
```

- [ ] **Step 6: Store** — `server/src/reports/mod.rs`:

```rust
//! Bug and crash reports: written by clients over HTTP, read only through the CLI.

pub mod routes;
pub mod store;
```

`server/src/reports/store.rs`:

```rust
use pulse_protocol::ids::{UserId, next_ulid};
use pulse_protocol::rest::{CreateReportRequest, ReportKind};
use sqlx::SqlitePool;

use crate::db::now;
use crate::error::{AppError, AppResult};

pub const MAX_TEXT_CHARS: usize = 4000;
pub const MAX_META_BYTES: usize = 16 * 1024;
pub const MAX_LOG_BYTES: usize = 512 * 1024;

pub fn validate(req: &CreateReportRequest) -> AppResult<()> {
    let bad = |m: &str| Err(AppError::BadRequest(m.into()));
    if req.kind == ReportKind::Bug && req.text.trim().is_empty() {
        return bad("say what happened");
    }
    if req.text.chars().count() > MAX_TEXT_CHARS {
        return bad("report text is too long (4000 characters max)");
    }
    if req.meta.to_string().len() > MAX_META_BYTES {
        return bad("report details are too large");
    }
    if req.log.len() > MAX_LOG_BYTES {
        return bad("report log is too large");
    }
    Ok(())
}

pub async fn insert(db: &SqlitePool, user: UserId, req: &CreateReportRequest) -> AppResult<String> {
    validate(req)?;
    let id = next_ulid().to_string();
    sqlx::query(
        "INSERT INTO bug_reports (id, user_id, created_at, kind, text, meta, log) VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(user.to_string())
    .bind(now())
    .bind(req.kind.as_str())
    .bind(req.text.trim())
    .bind(req.meta.to_string())
    .bind(&req.log)
    .execute(db)
    .await?;
    Ok(id)
}
```

- [ ] **Step 7: Route** — `server/src/reports/routes.rs`:

```rust
use axum::extract::{DefaultBodyLimit, State};
use axum::routing::post;
use axum::{Json, Router};
use pulse_protocol::rest::{CreateReportRequest, CreateReportResponse};

use crate::AppState;
use crate::auth::AuthUser;
use crate::error::{AppError, AppResult};

/// Room for a 512 KB log plus text and meta; everything else keeps axum's default.
const BODY_LIMIT: usize = 1024 * 1024;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/reports", post(create))
        .layer(DefaultBodyLimit::max(BODY_LIMIT))
}

async fn create(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Json(req): Json<CreateReportRequest>,
) -> AppResult<Json<CreateReportResponse>> {
    if !s.limits.report_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests);
    }
    let id = super::store::insert(&s.db, me, &req).await?;
    tracing::info!(%me, kind = req.kind.as_str(), %id, "report received");
    Ok(Json(CreateReportResponse { id }))
}
```

In `server/src/lib.rs`: `pub mod reports;` and `.merge(reports::routes::router())` after `voice::routes::router()`.

- [ ] **Step 8: Run, expect PASS** — `cargo test -p pulse-server --test reports` → 5 passed. If the 413 case comes back 400/422, check that the layer is on the reports router (it must wrap `/reports` before `merge`).

- [ ] **Step 9: Whole server suite** — `cargo test -p pulse-server` → all pass.

- [ ] **Step 10: Commit**

```bash
git add protocol server client/ui/src/lib/protocol
git commit -m "feat(server): POST /reports stores bug/crash reports (rate limited, size capped, write-only)"
```

---

### Task 3: Reading reports — CLI subcommands and `just` recipes

**Files:**
- Modify: `server/src/reports/store.rs` (list, find, mark seen)
- Modify: `server/src/main.rs` (`Reports`, `Report` subcommands)
- Modify: `justfile` (`reports`, `report` recipes)
- Test: `server/tests/reports.rs` (append)

**Interfaces:**
- Consumes: Task 2's table and `store::insert`.
- Produces: `store::Listed { id, short, created_at, username, kind, seen: bool, headline: String }`, `store::list(db, limit: i64) -> AppResult<Vec<Listed>>`, `store::Full { listed: Listed, text, meta: serde_json::Value, log }`, `store::find(db, short: &str) -> AppResult<Full>` (suffix match, case-insensitive; `NotFound`; ambiguous → `Conflict("…matches N reports…")`), `store::mark_seen(db, id: &str) -> AppResult<()>`, `store::short(id: &str) -> String` (last 6 chars).

- [ ] **Step 1: Write the failing tests** — append to `server/tests/reports.rs`:

```rust
use pulse_server::reports::store;

async fn file(app: &TestApp, token: &str, kind: &str, text: &str, meta: serde_json::Value) -> String {
    let body = json!({ "kind": kind, "text": text, "meta": meta, "log": "the log\n" });
    let r = post_json(app, token, "/reports", body).await;
    r.json::<serde_json::Value>().await.unwrap()["id"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn list_is_newest_first_with_unseen_marked() {
    let app = spawn().await;
    let (_, token) = register(&app, "alex").await;
    let first = file(&app, &token, "bug", "first one", json!({})).await;
    let second = file(&app, &token, "crash", "", json!({ "crash": { "message": "index out of bounds" } })).await;
    let list = store::list(&app.db, 20).await.unwrap();
    assert_eq!(list.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), [second.as_str(), first.as_str()]);
    assert_eq!(list[0].username, "alex");
    assert_eq!(list[0].kind, "crash");
    assert_eq!(list[0].headline, "index out of bounds", "crashes headline the panic message");
    assert_eq!(list[1].headline, "first one");
    assert!(list.iter().all(|r| !r.seen));
    assert_eq!(list[0].short, store::short(&second));
}

#[tokio::test]
async fn find_by_short_id_and_mark_seen() {
    let app = spawn().await;
    let (_, token) = register(&app, "alex").await;
    let id = file(&app, &token, "bug", "voice cut out", json!({ "os": "Windows 11" })).await;
    let full = store::find(&app.db, &store::short(&id).to_lowercase()).await.unwrap();
    assert_eq!(full.listed.id, id);
    assert_eq!(full.text, "voice cut out");
    assert_eq!(full.meta["os"], "Windows 11");
    assert_eq!(full.log, "the log\n");
    store::mark_seen(&app.db, &id).await.unwrap();
    assert!(store::list(&app.db, 20).await.unwrap()[0].seen);
    assert!(matches!(store::find(&app.db, "ZZZZZZ").await, Err(pulse_server::error::AppError::NotFound)));
}

#[tokio::test]
async fn ambiguous_suffix_is_an_error() {
    let app = spawn().await;
    let (_, token) = register(&app, "alex").await;
    file(&app, &token, "bug", "a", json!({})).await;
    file(&app, &token, "bug", "b", json!({})).await;
    // "" matches every report
    assert!(matches!(store::find(&app.db, "").await, Err(pulse_server::error::AppError::Conflict(_))));
}
```

- [ ] **Step 2: Run, expect FAIL** — `cargo test -p pulse-server --test reports` → compile errors (`store::list` etc. missing).

- [ ] **Step 3: Implement** — append to `server/src/reports/store.rs`:

```rust
#[derive(Debug, Clone)]
pub struct Listed {
    pub id: String,
    pub short: String,
    pub created_at: String,
    pub username: String,
    pub kind: String,
    pub seen: bool,
    pub headline: String,
}

#[derive(Debug, Clone)]
pub struct Full {
    pub listed: Listed,
    pub text: String,
    pub meta: serde_json::Value,
    pub log: String,
}

/// What lists show as a report's id: the random tail of the ULID (its head is a timestamp, so
/// reports sent close together would share a prefix).
pub fn short(id: &str) -> String {
    id[id.len().saturating_sub(6)..].to_string()
}

type Row = (String, String, String, String, Option<String>, String, String, String);

fn listed(r: &Row) -> Listed {
    let (id, created_at, username, kind, seen_at, text, meta, _) = r;
    let meta: serde_json::Value = serde_json::from_str(meta).unwrap_or_default();
    let headline = meta["crash"]["message"]
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| text.clone());
    Listed {
        id: id.clone(),
        short: short(id),
        created_at: created_at.clone(),
        username: username.clone(),
        kind: kind.clone(),
        seen: seen_at.is_some(),
        headline: headline.lines().next().unwrap_or_default().chars().take(60).collect(),
    }
}

const SELECT: &str = "SELECT r.id, r.created_at, u.username, r.kind, r.seen_at, r.text, r.meta, r.log
     FROM bug_reports r JOIN users u ON u.id = r.user_id";

pub async fn list(db: &SqlitePool, limit: i64) -> AppResult<Vec<Listed>> {
    let rows: Vec<Row> = sqlx::query_as(&format!("{SELECT} ORDER BY r.id DESC LIMIT ?"))
        .bind(limit)
        .fetch_all(db)
        .await?;
    Ok(rows.iter().map(listed).collect())
}

pub async fn find(db: &SqlitePool, short_id: &str) -> AppResult<Full> {
    let rows: Vec<Row> = sqlx::query_as(&format!("{SELECT} WHERE r.id LIKE '%' || ? ORDER BY r.id DESC"))
        .bind(short_id.to_uppercase())
        .fetch_all(db)
        .await?;
    match rows.len() {
        0 => Err(AppError::NotFound),
        1 => {
            let r = &rows[0];
            Ok(Full {
                listed: listed(r),
                text: r.5.clone(),
                meta: serde_json::from_str(&r.6).unwrap_or_default(),
                log: r.7.clone(),
            })
        }
        n => Err(AppError::Conflict(format!("'{short_id}' matches {n} reports; use more characters"))),
    }
}

pub async fn mark_seen(db: &SqlitePool, id: &str) -> AppResult<()> {
    sqlx::query("UPDATE bug_reports SET seen_at = COALESCE(seen_at, ?) WHERE id = ?")
        .bind(now())
        .bind(id)
        .execute(db)
        .await?;
    Ok(())
}
```

- [ ] **Step 4: Run, expect PASS** — `cargo test -p pulse-server --test reports` → 8 passed.

- [ ] **Step 5: CLI** — `server/src/main.rs`, add to `enum Cmd`:

```rust
    /// List the newest bug/crash reports
    Reports {
        #[arg(long, default_value_t = 20)]
        limit: i64,
    },
    /// Show one report (by the short id from `reports`) and mark it seen
    Report {
        id: String,
        /// Also print the attached app log
        #[arg(long)]
        log: bool,
    },
```

and the arms:

```rust
        Cmd::Reports { limit } => {
            let db = pulse_server::db::connect(&cfg.db_url).await?;
            let rows = pulse_server::reports::store::list(&db, limit).await?;
            if rows.is_empty() {
                println!("no reports");
            }
            for r in rows {
                println!(
                    "{}  {}  {:<16} {:<15} {:3}  {}",
                    r.short,
                    &r.created_at[..16],
                    r.username,
                    r.kind,
                    if r.seen { "" } else { "new" },
                    r.headline
                );
            }
            Ok(())
        }
        Cmd::Report { id, log } => {
            let db = pulse_server::db::connect(&cfg.db_url).await?;
            let r = pulse_server::reports::store::find(&db, &id).await?;
            let l = &r.listed;
            println!("#{}  {}  {}  {}\n", l.short, l.created_at, l.username, l.kind);
            println!("{}\n", if r.text.is_empty() { "(no text)" } else { &r.text });
            println!("{}", serde_json::to_string_pretty(&r.meta)?);
            if log {
                println!("\n----- app log -----\n{}", r.log);
            } else {
                println!("\n({} KB of log: add --log)", r.log.len() / 1024);
            }
            pulse_server::reports::store::mark_seen(&db, &l.id).await?;
            Ok(())
        }
```

(`AppError` converts into `anyhow::Error` via `?` because it is `std::error::Error + Send + Sync`; if the compiler says otherwise, map with `.map_err(|e| anyhow::anyhow!(e.to_string()))`.) Check `serde_json` is a server dependency (it is: `server/Cargo.toml`).

- [ ] **Step 6: `just` recipes** — after `invite-prod` in `justfile`:

```just
# newest bug/crash reports on the live server (`new` = not looked at yet)
reports:
    ssh {{prod}} 'cd /srv/pulse && docker compose exec -T pulse-server pulse-app-server reports'

# one report in full; add --log for the attached app log
report id *flags:
    ssh {{prod}} 'cd /srv/pulse && docker compose exec -T pulse-server pulse-app-server report {{id}} {{flags}}'
```

Local check: `cargo run -q -p pulse-server --bin pulse-app-server -- reports` (against the dev DB) → prints `no reports` or a list.

- [ ] **Step 7: Commit**

```bash
git add server justfile
git commit -m "feat(server): reports/report CLI and just recipes to read bug reports"
```

---

### Task 4: Client — gather, send (retry once), save to file

**Files:**
- Create: `client/src-tauri/src/reports/mod.rs` (gather + send + save), `client/src-tauri/src/reports/tail.rs`
- Modify: `client/src-tauri/src/lib.rs` (`pub mod reports;`, `app.manage(reports::LogDir(log_dir.clone()))`, register commands `submit_report`, `save_report`)
- Modify: `client/src-tauri/src/commands.rs` (`Core.conn: Mutex<ConnState>` set in `forward`, `pub fn conn(&self) -> ConnState`)
- Modify: `client/src-tauri/src/voice/mod.rs` (`pub async fn diagnostics(&self) -> serde_json::Value`)
- Modify: `client/src-tauri/src/api.rs` (`create_report`)
- Modify: `client/src-tauri/Cargo.toml` (`os_info = "3"`), `client/src-tauri/build.rs`, `.github/workflows/release-windows.yml`

**Interfaces:**
- Consumes: Task 2's `CreateReportRequest`, `CreateReportResponse`, `ReportKind`.
- Produces: `reports::LogDir(pub PathBuf)` (managed state), `reports::tail::log_tail(dir: &Path, max: usize) -> String`, `reports::TAIL_BYTES: usize = 256 * 1024`, `reports::send_with_retry<F, Fut>(attempt: F, delay: Duration) -> Result<CreateReportResponse, ApiError>`, `reports::meta(app: &AppHandle, server: &str) -> serde_json::Value` (async), `reports::file_text(kind, text, meta, log) -> String`, `reports::save_dir(app) -> PathBuf`, `reports::short(id) -> String`.
- Produces (commands): `submit_report(text: String) -> Result<String /*short id*/, ApiError>`, `save_report(text: String, server_url: Option<String>) -> Result<String /*path*/, String>`.

- [ ] **Step 1: Failing tail tests** — `client/src-tauri/src/reports/tail.rs`:

```rust
//! The end of the newest log file, for attaching to a report.

use std::path::Path;

/// Last `max` bytes of the newest `pulse.*.log` in `dir`, cut to start at a whole line and
/// decoded lossily (a cut can land mid-character). Empty if there's no log.
pub fn log_tail(dir: &Path, max: usize) -> String {
    let _ = (dir, max);
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_newest_file() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("pulse.2026-10-11.log"), "old\n").unwrap();
        std::fs::write(d.path().join("pulse.2026-10-12.log"), "new\n").unwrap();
        std::fs::write(d.path().join("crash.json"), "{}").unwrap();
        assert_eq!(log_tail(d.path(), 1024), "new\n");
    }

    #[test]
    fn tail_starts_at_a_whole_line() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("pulse.2026-10-12.log"), "first line\nsecond é line\nthird\n").unwrap();
        // 20 bytes from the end lands inside "second é line"
        assert_eq!(log_tail(d.path(), 20), "third\n");
    }

    #[test]
    fn no_log_is_empty() {
        let d = tempfile::tempdir().unwrap();
        assert_eq!(log_tail(d.path(), 1024), "");
        assert_eq!(log_tail(&d.path().join("missing"), 1024), "");
    }
}
```

Create `client/src-tauri/src/reports/mod.rs` with `pub mod tail;` and add `pub mod reports;` to `lib.rs`.

- [ ] **Step 2: Run, expect FAIL** — `cargo test -p pulse-client --lib reports::tail` → `reads_the_newest_file` and `tail_starts_at_a_whole_line` FAIL.

- [ ] **Step 3: Implement**

```rust
pub fn log_tail(dir: &Path, max: usize) -> String {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(entries) = std::fs::read_dir(dir) else {
        return String::new();
    };
    let newest = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("pulse.") && n.ends_with(".log"))
        })
        .max(); // names are pulse.YYYY-MM-DD.log: the greatest is the newest
    let Some(path) = newest else {
        return String::new();
    };
    let Ok(mut f) = std::fs::File::open(&path) else {
        return String::new();
    };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    let start = len.saturating_sub(max as u64);
    let mut buf = Vec::new();
    if f.seek(SeekFrom::Start(start)).is_err() || f.read_to_end(&mut buf).is_err() {
        return String::new();
    }
    if start > 0 {
        // drop the partial first line
        match buf.iter().position(|b| *b == b'\n') {
            Some(i) => {
                buf.drain(..=i);
            }
            None => buf.clear(),
        }
    }
    String::from_utf8_lossy(&buf).into_owned()
}
```

- [ ] **Step 4: Run, expect PASS** — `cargo test -p pulse-client --lib reports::tail` → 3 passed.

- [ ] **Step 5: Failing tests for retry, file text, short id** — `client/src-tauri/src/reports/mod.rs`:

```rust
//! Bug and crash reports: gather diagnostics, send them, or save them to a file to pass on by hand.

pub mod tail;

use std::future::Future;
use std::path::PathBuf;
use std::time::Duration;

use pulse_protocol::rest::{CreateReportResponse, ReportKind};

use crate::api::ApiError;

/// The profile's log dir (reports read their log tail from it; crash markers live there).
pub struct LogDir(pub PathBuf);

/// How much of the log a report carries.
pub const TAIL_BYTES: usize = 256 * 1024;

/// Try, and on a network or server error try once more after `delay`. Refusals (4xx, 429) aren't
/// retried: sending the same thing again won't change the answer.
pub async fn send_with_retry<F, Fut>(mut attempt: F, delay: Duration) -> Result<CreateReportResponse, ApiError>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<CreateReportResponse, ApiError>>,
{
    let _ = delay;
    attempt().await
}

pub fn short(id: &str) -> String {
    id[id.len().saturating_sub(6)..].to_string()
}

/// The saved-file form of a report: readable as-is in Notepad / WhatsApp.
pub fn file_text(kind: ReportKind, text: &str, meta: &serde_json::Value, log: &str) -> String {
    let _ = (kind, text, meta, log);
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn ok() -> Result<CreateReportResponse, ApiError> {
        Ok(CreateReportResponse { id: "01JABCDEFGHJKMNPQRSTVWXYZ".into() })
    }

    #[tokio::test]
    async fn a_network_error_is_retried_once() {
        let calls = AtomicU32::new(0);
        let r = send_with_retry(
            || {
                let n = calls.fetch_add(1, Ordering::SeqCst);
                async move { if n == 0 { Err(ApiError::Network("blip".into())) } else { ok() } }
            },
            Duration::from_millis(1),
        )
        .await;
        assert!(r.is_ok());
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn gives_up_after_the_second_failure() {
        let calls = AtomicU32::new(0);
        let r = send_with_retry(
            || {
                calls.fetch_add(1, Ordering::SeqCst);
                async { Err(ApiError::Server("502".into())) }
            },
            Duration::from_millis(1),
        )
        .await;
        assert!(matches!(r, Err(ApiError::Server(_))));
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn a_refusal_is_not_retried() {
        let calls = AtomicU32::new(0);
        let r = send_with_retry(
            || {
                calls.fetch_add(1, Ordering::SeqCst);
                async { Err(ApiError::Rejected("say what happened".into())) }
            },
            Duration::from_millis(1),
        )
        .await;
        assert!(matches!(r, Err(ApiError::Rejected(_))));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn file_text_has_the_text_details_and_log() {
        let meta = serde_json::json!({ "app_version": "0.1.0", "os": "Windows 11" });
        let t = file_text(ReportKind::Bug, "voice cut out", &meta, "log line\n");
        assert!(t.starts_with("Pulse bug report\n"), "{t}");
        assert!(t.contains("voice cut out"));
        assert!(t.contains("\"os\": \"Windows 11\""));
        assert!(t.ends_with("----- app log -----\nlog line\n"), "{t}");
    }

    #[test]
    fn short_is_the_last_six() {
        assert_eq!(short("01JABCDEFGHJKMNPQRSTVWXYZ"), "TVWXYZ");
    }
}
```

- [ ] **Step 6: Run, expect FAIL** — `cargo test -p pulse-client --lib reports::tests` → `a_network_error_is_retried_once`, `gives_up_after_the_second_failure`, `file_text_…` FAIL.

- [ ] **Step 7: Implement**

```rust
pub async fn send_with_retry<F, Fut>(mut attempt: F, delay: Duration) -> Result<CreateReportResponse, ApiError>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<CreateReportResponse, ApiError>>,
{
    match attempt().await {
        Err(ApiError::Network(_) | ApiError::Server(_)) => {
            tokio::time::sleep(delay).await;
            attempt().await
        }
        r => r,
    }
}

pub fn file_text(kind: ReportKind, text: &str, meta: &serde_json::Value, log: &str) -> String {
    let title = match kind {
        ReportKind::Bug => "Pulse bug report",
        ReportKind::Crash => "Pulse crash report",
        ReportKind::UnexpectedExit => "Pulse unexpected-exit report",
    };
    format!(
        "{title}\n\n{}\n\n{}\n\n----- app log -----\n{log}",
        if text.trim().is_empty() { "(no text)" } else { text.trim() },
        serde_json::to_string_pretty(meta).unwrap_or_default(),
    )
}
```

- [ ] **Step 8: Run, expect PASS** — `cargo test -p pulse-client --lib reports` → 8 passed.

- [ ] **Step 9: Connection state in `Core`** — in `commands.rs`, add a field `conn: Mutex<ConnState>` (init `ConnState::Connecting` in `Core::new`), a getter:

```rust
    pub(crate) fn conn(&self) -> ConnState {
        *self.conn.lock().unwrap()
    }
```

and in `forward`, inside `GatewayUpdate::Connection(state)` first line: `*app.state::<Core>().conn.lock().unwrap() = state;`.

- [ ] **Step 10: Voice diagnostics** — in `voice/mod.rs` `impl VoiceManager`:

```rust
    /// For bug reports: where voice is and which devices it was told to use.
    pub async fn diagnostics(&self) -> serde_json::Value {
        let s = self.session.lock().await;
        let (channel, cfg) = match s.as_ref() {
            Some(s) if s.alive.load(Ordering::SeqCst) => (Some(s.channel.to_string()), Some(s.cfg.lock().unwrap().clone())),
            _ => (None, None),
        };
        serde_json::json!({
            "channel": channel,
            "controls": self.controls(),
            "input": cfg.as_ref().and_then(|c| c.input.clone()).unwrap_or_else(|| "default".into()),
            "output": cfg.as_ref().and_then(|c| c.output.clone()).unwrap_or_else(|| "default".into()),
        })
    }
```

- [ ] **Step 11: Api method** — in `api.rs` `impl Api` (import `CreateReportRequest, CreateReportResponse`):

```rust
    /// Reports carry up to 256 KB of log: allow longer than the usual 15 s on a slow line.
    pub async fn create_report(&self, token: &str, req: &CreateReportRequest) -> Result<CreateReportResponse, ApiError> {
        Self::parse(
            self.http
                .post(format!("{}/reports", self.base))
                .bearer_auth(token)
                .timeout(Duration::from_secs(45))
                .json(req)
                .send()
                .await,
        )
        .await
    }
```

Test in `api.rs` tests:

```rust
    #[tokio::test]
    async fn a_report_reaches_the_server() {
        let app = testing::spawn().await;
        let (_, token) = testing::register(&app, "alex").await;
        let api = Api::new(&format!("http://{}", app.addr));
        let req = pulse_protocol::rest::CreateReportRequest {
            kind: pulse_protocol::rest::ReportKind::Bug,
            text: "voice cut out".into(),
            meta: serde_json::json!({}),
            log: String::new(),
        };
        assert_eq!(api.create_report(&token, &req).await.unwrap().id.len(), 26);
    }
```

Run: `cargo test -p pulse-client --lib a_report_reaches_the_server` → PASS.

- [ ] **Step 12: Build info** — `client/src-tauri/build.rs`:

```rust
fn main() {
    // Release workflows set PULSE_COMMIT (from GITHUB_SHA); bug reports name the build.
    println!("cargo:rerun-if-env-changed=PULSE_COMMIT");
    tauri_build::build()
}
```

In `.github/workflows/release-windows.yml` top-level `env:` add `PULSE_COMMIT: ${{ github.sha }}`. Add `os_info = "3"` to `client/src-tauri/Cargo.toml` `[dependencies]`.

- [ ] **Step 13: Meta, save dir, commands** — append to `reports/mod.rs`:

```rust
use tauri::{AppHandle, Manager};

use crate::commands::Core;
use crate::voice::VoiceManager;

/// Everything a report carries besides the text and log.
pub async fn meta(app: &AppHandle, server: &str) -> serde_json::Value {
    let commit = option_env!("PULSE_COMMIT").map(|c| &c[..c.len().min(7)]).unwrap_or("dev");
    serde_json::json!({
        "app_version": env!("CARGO_PKG_VERSION"),
        "commit": commit,
        "os": os_info::get().to_string(),
        "server": server,
        "gateway": format!("{:?}", app.state::<Core>().conn()),
        "voice": app.state::<VoiceManager>().diagnostics().await,
    })
}

/// Where "Save report" writes: Downloads, else Desktop, else the log dir.
pub fn save_dir(app: &AppHandle) -> PathBuf {
    let p = app.path();
    p.download_dir()
        .or_else(|_| p.desktop_dir())
        .unwrap_or_else(|_| app.state::<LogDir>().0.clone())
}

```

and the file name (the client has no date crate, so compute UTC from unix time):

```rust
/// `pulse-report-YYYYMMDD-HHMMSS.txt` in UTC, from the unix time (no date crate in the client).
pub fn file_name(unix_secs: u64) -> String {
    let (days, secs) = (unix_secs / 86_400, unix_secs % 86_400);
    // civil-from-days (Howard Hinnant), valid for 1970+
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "pulse-report-{y:04}{m:02}{d:02}-{:02}{:02}{:02}.txt",
        secs / 3600,
        secs % 3600 / 60,
        secs % 60
    )
}

pub fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
```

with a test:

```rust
    #[test]
    fn file_name_is_utc_date_and_time() {
        // 2026-10-12 13:25:07 UTC
        assert_eq!(file_name(1_791_811_507), "pulse-report-20261012-132507.txt");
    }
```

Run it red first (stub returning `String::new()`), then with the implementation green: `cargo test -p pulse-client --lib file_name_is_utc` → PASS.

Then the commands, in `reports/mod.rs`:

```rust
use pulse_protocol::rest::CreateReportRequest;

#[tauri::command]
pub async fn submit_report(app: AppHandle, text: String) -> Result<String, ApiError> {
    let (api, token) = app.state::<Core>().current()?;
    let req = CreateReportRequest {
        kind: ReportKind::Bug,
        text,
        meta: meta(&app, api.base()).await,
        log: tail::log_tail(&app.state::<LogDir>().0, TAIL_BYTES),
    };
    let r = send_with_retry(|| api.create_report(&token, &req), Duration::from_secs(2)).await;
    let r = app.state::<Core>().check(&app, r)?;
    tracing::info!(id = %r.id, "bug report sent");
    Ok(short(&r.id))
}

#[tauri::command]
pub async fn save_report(app: AppHandle, text: String, server_url: Option<String>) -> Result<String, String> {
    let server = match server_url {
        Some(s) => s,
        None => app.state::<Core>().current().map(|(a, _)| a.base().to_string()).unwrap_or_default(),
    };
    let meta = meta(&app, &server).await;
    let log = tail::log_tail(&app.state::<LogDir>().0, TAIL_BYTES);
    let path = save_dir(&app).join(file_name(unix_now()));
    std::fs::write(&path, file_text(ReportKind::Bug, &text, &meta, &log)).map_err(|e| e.to_string())?;
    tracing::info!(path = %path.display(), "bug report saved");
    Ok(path.display().to_string())
}
```

(`Core::current` and `Core::check` are `pub(crate)`: fine from `crate::reports`.)

In `lib.rs` setup, right after computing `log_dir`: `app.manage(reports::LogDir(log_dir.clone()));`. In `generate_handler!` add `reports::submit_report, reports::save_report,`.

- [ ] **Step 14: Run** — `cargo test -p pulse-client` → all pass; `cargo clippy -p pulse-client --all-targets -- -D warnings` → clean.

- [ ] **Step 15: Commit**

```bash
git add client/src-tauri .github/workflows/release-windows.yml Cargo.lock
git commit -m "feat(client): gather + send bug reports (retry once) or save them to Downloads"
```

---

### Task 5: Automatic crash and unexpected-exit reports

**Files:**
- Create: `client/src-tauri/src/reports/crash.rs`
- Modify: `client/src-tauri/src/reports/mod.rs` (`pub mod crash;`)
- Modify: `client/src-tauri/src/logging.rs` (panic hook writes the marker)
- Modify: `client/src-tauri/src/lib.rs` (detect before `logging::init`, mark running, clear on exit, manage `crash::Pending`)
- Modify: `client/src-tauri/src/commands.rs` (`Core::activate` spawns `crash::send_pending`)

**Interfaces:**
- Consumes: `tail::log_tail`, `TAIL_BYTES`, `meta`, `send_with_retry`, `LogDir`, `unix_now`, `Api::create_report`.
- Produces: `crash::write_panic_marker(dir: &Path, message: &str, location: &str)`, `crash::detect(dir: &Path, now: u64)` (turns markers into `pending-report.json`), `crash::mark_running(dir: &Path)`, `crash::clear_running(dir: &Path)`, `crash::PendingFile { at: u64, kind: ReportKind, crash: Option<serde_json::Value>, log: String }`, `crash::load_pending(dir: &Path, now: u64) -> Option<PendingFile>`, `crash::Outcome { Sent, Keep, Drop }`, `crash::outcome(r: &Result<_, ApiError>) -> Outcome`, `crash::send_pending(app: AppHandle, api: Api, token: String)`; event `pulse://auto-report-sent`.

- [ ] **Step 1: Failing tests** — `client/src-tauri/src/reports/crash.rs`:

```rust
//! Crashes and unexpected exits, reported on the next launch.
//!
//! Files in the profile's log dir:
//! - `running`: written at startup, removed on a clean exit. Still there at startup → the last
//!   session ended without one (native crash, force-close, power loss).
//! - `crash.json`: written by the panic hook (first panic of a session).
//! - `pending-report.json`: what startup made of the two above, plus the crashed session's log
//!   tail; kept until it is sent (or rejected, or a week old).

use std::path::Path;

use pulse_protocol::rest::ReportKind;
use serde::{Deserialize, Serialize};

use crate::api::ApiError;

const RUNNING: &str = "running";
const CRASH: &str = "crash.json";
const PENDING: &str = "pending-report.json";
pub const MAX_AGE_SECS: u64 = 7 * 86_400;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PendingFile {
    pub at: u64,
    pub kind: ReportKind,
    pub crash: Option<serde_json::Value>,
    pub log: String,
}

#[derive(Debug, PartialEq)]
pub enum Outcome {
    Sent,
    Keep,
    Drop,
}

pub fn write_panic_marker(dir: &Path, message: &str, location: &str) {
    let _ = (dir, message, location);
}
pub fn mark_running(dir: &Path) {
    let _ = dir;
}
pub fn clear_running(dir: &Path) {
    let _ = dir;
}
pub fn detect(dir: &Path, now: u64) {
    let _ = (dir, now);
}
pub fn load_pending(dir: &Path, now: u64) -> Option<PendingFile> {
    let _ = (dir, now);
    None
}
pub fn clear_pending(dir: &Path) {
    let _ = dir;
}
pub fn outcome<T>(r: &Result<T, ApiError>) -> Outcome {
    let _ = r;
    Outcome::Keep
}

#[cfg(test)]
mod tests {
    use super::*;

    fn log(dir: &Path, text: &str) {
        std::fs::write(dir.join("pulse.2026-10-12.log"), text).unwrap();
    }

    #[test]
    fn clean_exit_leaves_nothing_to_report() {
        let d = tempfile::tempdir().unwrap();
        mark_running(d.path());
        clear_running(d.path());
        detect(d.path(), 1000);
        assert_eq!(load_pending(d.path(), 1000), None);
    }

    #[test]
    fn a_leftover_running_marker_is_an_unexpected_exit_with_the_old_log() {
        let d = tempfile::tempdir().unwrap();
        log(d.path(), "joining voice\n");
        mark_running(d.path());
        detect(d.path(), 1000);
        let p = load_pending(d.path(), 1000).unwrap();
        assert_eq!(p.kind, ReportKind::UnexpectedExit);
        assert_eq!(p.log, "joining voice\n");
        assert!(!d.path().join(RUNNING).exists(), "markers are consumed");
    }

    #[test]
    fn a_panic_marker_wins_over_running_and_the_first_panic_is_kept() {
        let d = tempfile::tempdir().unwrap();
        mark_running(d.path());
        write_panic_marker(d.path(), "index out of bounds", "src/voice/mixer.rs:88");
        write_panic_marker(d.path(), "second panic", "elsewhere.rs:1");
        detect(d.path(), 1000);
        let p = load_pending(d.path(), 1000).unwrap();
        assert_eq!(p.kind, ReportKind::Crash);
        let c = p.crash.unwrap();
        assert_eq!(c["message"], "index out of bounds");
        assert_eq!(c["location"], "src/voice/mixer.rs:88");
    }

    #[test]
    fn panic_marker_write_ignores_a_missing_dir() {
        let d = tempfile::tempdir().unwrap();
        write_panic_marker(&d.path().join("gone/away"), "boom", "x.rs:1"); // must not panic
    }

    #[test]
    fn a_week_old_pending_report_is_dropped() {
        let d = tempfile::tempdir().unwrap();
        mark_running(d.path());
        detect(d.path(), 1000);
        assert_eq!(load_pending(d.path(), 1000 + MAX_AGE_SECS + 1), None);
        assert!(!d.path().join(PENDING).exists());
    }

    #[test]
    fn markers_are_per_dir() {
        let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        mark_running(a.path());
        mark_running(b.path());
        clear_running(b.path()); // profile B exits cleanly
        detect(a.path(), 1000);
        detect(b.path(), 1000);
        assert!(load_pending(a.path(), 1000).is_some());
        assert!(load_pending(b.path(), 1000).is_none());
    }

    #[test]
    fn pending_dropped_on_rejection_kept_on_network_error() {
        let ok: Result<(), ApiError> = Ok(());
        assert_eq!(outcome(&ok), Outcome::Sent);
        assert_eq!(outcome::<()>(&Err(ApiError::Rejected("too large".into()))), Outcome::Drop);
        assert_eq!(outcome::<()>(&Err(ApiError::Network("down".into()))), Outcome::Keep);
        assert_eq!(outcome::<()>(&Err(ApiError::Server("502".into()))), Outcome::Keep);
        assert_eq!(outcome::<()>(&Err(ApiError::RateLimited)), Outcome::Keep);
        assert_eq!(outcome::<()>(&Err(ApiError::Unauthorized)), Outcome::Keep);
    }
}
```

Add `pub mod crash;` to `reports/mod.rs`.

- [ ] **Step 2: Run, expect FAIL** — `cargo test -p pulse-client --lib reports::crash` → 5 of 7 FAIL (`clean_exit…` and `panic_marker_write_ignores…` pass on the stubs).

- [ ] **Step 3: Implement** (replace the stubs):

```rust
pub fn write_panic_marker(dir: &Path, message: &str, location: &str) {
    let path = dir.join(CRASH);
    if path.exists() {
        return; // the first panic of a session is the interesting one
    }
    let body = serde_json::json!({
        "at": super::unix_now(),
        "message": message,
        "location": location,
        "thread": std::thread::current().name().unwrap_or("?"),
    });
    let _ = std::fs::write(path, body.to_string()); // in a panic hook: never fail loudly
}

pub fn mark_running(dir: &Path) {
    let _ = std::fs::create_dir_all(dir);
    let _ = std::fs::write(dir.join(RUNNING), super::unix_now().to_string());
}

pub fn clear_running(dir: &Path) {
    let _ = std::fs::remove_file(dir.join(RUNNING));
}

/// At startup, before this session logs anything: turn last session's markers into a pending
/// report carrying its log tail. A newer one replaces an older unsent one.
pub fn detect(dir: &Path, now: u64) {
    let crash: Option<serde_json::Value> = std::fs::read_to_string(dir.join(CRASH))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok());
    let ran = dir.join(RUNNING).exists();
    let kind = match (&crash, ran) {
        (Some(_), _) => ReportKind::Crash,
        (None, true) => ReportKind::UnexpectedExit,
        (None, false) => return,
    };
    let pending = PendingFile {
        at: now,
        kind,
        crash,
        log: super::tail::log_tail(dir, super::TAIL_BYTES),
    };
    if let Ok(body) = serde_json::to_string(&pending) {
        let _ = std::fs::write(dir.join(PENDING), body);
    }
    let _ = std::fs::remove_file(dir.join(CRASH));
    clear_running(dir);
}

pub fn load_pending(dir: &Path, now: u64) -> Option<PendingFile> {
    let p: PendingFile = serde_json::from_str(&std::fs::read_to_string(dir.join(PENDING)).ok()?).ok()?;
    if now.saturating_sub(p.at) > MAX_AGE_SECS {
        clear_pending(dir);
        return None;
    }
    Some(p)
}

pub fn clear_pending(dir: &Path) {
    let _ = std::fs::remove_file(dir.join(PENDING));
}

/// Sent → delete. Refused (4xx) → delete too: resending won't change the answer. Otherwise keep it
/// for the next launch.
pub fn outcome<T>(r: &Result<T, ApiError>) -> Outcome {
    match r {
        Ok(_) => Outcome::Sent,
        Err(ApiError::Rejected(_)) => Outcome::Drop,
        Err(_) => Outcome::Keep,
    }
}
```

`ReportKind` needs `PartialEq` (it has it) and `Serialize/Deserialize` (it has them).

- [ ] **Step 4: Run, expect PASS** — `cargo test -p pulse-client --lib reports::crash` → 7 passed.

- [ ] **Step 5: Sending** — append to `crash.rs`:

```rust
use pulse_protocol::rest::CreateReportRequest;
use tauri::{AppHandle, Emitter, Manager};

/// After sign-in: send last session's crash/exit report, if any.
pub async fn send_pending(app: AppHandle, api: crate::api::Api, token: String) {
    let dir = app.state::<super::LogDir>().0.clone();
    let Some(p) = load_pending(&dir, super::unix_now()) else {
        return;
    };
    let mut meta = super::meta(&app, api.base()).await;
    meta["crash"] = p.crash.clone().unwrap_or(serde_json::Value::Null);
    let req = CreateReportRequest { kind: p.kind, text: String::new(), meta, log: p.log };
    let r = super::send_with_retry(|| api.create_report(&token, &req), std::time::Duration::from_secs(2)).await;
    match outcome(&r) {
        Outcome::Sent => {
            clear_pending(&dir);
            tracing::info!(kind = req.kind.as_str(), "automatic report sent");
            let _ = app.emit("pulse://auto-report-sent", ());
        }
        Outcome::Drop => {
            clear_pending(&dir);
            tracing::warn!(error = ?r.err(), "automatic report refused; dropped");
        }
        Outcome::Keep => tracing::warn!(error = ?r.err(), "automatic report not sent; will retry next launch"),
    }
}
```

In `commands.rs` `Core::activate`, after the gateway spawn:

```rust
        tauri::async_runtime::spawn(crate::reports::crash::send_pending(app.clone(), api.clone(), token.clone()));
```

(Move this line above any statement that moves `api`/`token`; `activate` uses `token` in `GatewayHandle::spawn`, so clone before it.)

- [ ] **Step 6: Panic hook and startup/exit wiring** — `logging::init` gets the dir already; change its panic hook to:

```rust
    let marker_dir = dir.to_path_buf();
    std::panic::set_hook(Box::new(move |info| {
        tracing::error!(panic = %info, "PANIC");
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "panic".into());
        let location = info.location().map(|l| format!("{}:{}", l.file(), l.line())).unwrap_or_default();
        crate::reports::crash::write_panic_marker(&marker_dir, &message, &location);
    }));
```

In `lib.rs` setup, **before** `logging::init(&log_dir)`:

```rust
            // Last session's crash / unexpected exit, captured before this session writes to the log.
            reports::crash::detect(&log_dir, reports::unix_now());
            reports::crash::mark_running(&log_dir);
            app.manage(reports::LogDir(log_dir.clone()));
```

(and remove the `app.manage(reports::LogDir(...))` added in Task 4 if it sits elsewhere — one `manage` only). In the `RunEvent::Exit` arm, after the voice leave block:

```rust
                reports::crash::clear_running(&app.state::<reports::LogDir>().0);
```

- [ ] **Step 7: Run** — `cargo test -p pulse-client` → all pass; `cargo clippy -p pulse-client --all-targets -- -D warnings` → clean.

- [ ] **Step 8: Manual check (Linux, dev server)** — `just dev-server` in one terminal; `cargo run -p pulse-client` (or `cargo tauri dev`), sign in, then `pkill -9 pulse-app`. Relaunch and sign in → server log shows `report received … kind="unexpected-exit"`; `cargo run -q -p pulse-server --bin pulse-app-server -- reports` lists it. Relaunch once more → nothing new is sent.

- [ ] **Step 9: Commit**

```bash
git add client/src-tauri
git commit -m "feat(client): report panics and unexpected exits automatically on the next sign-in"
```

---

### Task 6: UI — Report a bug modal, menu item, login link, crash notice

**Files:**
- Create: `client/ui/src/lib/report.ts`, `client/ui/src/lib/report.test.ts`, `client/ui/src/components/ReportBug.svelte`
- Modify: `client/ui/src/lib/tauri.ts` (`submitReport`, `saveReport`)
- Modify: `client/ui/src/app.css` (`--warn` token)
- Modify: `client/ui/src/routes/Shell.svelte` (menu item, modal, crash notice)
- Modify: `client/ui/src/routes/Login.svelte` (link + modal in save-only mode)

**Interfaces:**
- Consumes: commands `submit_report(text) -> short id`, `save_report(text, serverUrl?) -> path`; event `pulse://auto-report-sent`.
- Produces: `report.ts`: `type ReportState = { step: 'edit' | 'sending' | 'sent' | 'failed' | 'saving' | 'saved'; message: string }`, `submitFlow(text, deps)`, `saveFlow(text, deps)`.

- [ ] **Step 1: Failing flow tests** — `client/ui/src/lib/report.test.ts`:

```ts
import { describe, expect, it } from 'vitest'
import { saveFlow, submitFlow } from './report'

describe('report flow', () => {
  it('sent: thanks with the short id', async () => {
    const steps: string[] = []
    const s = await submitFlow('voice cut out', { submit: async () => 'TVWXYZ', onStep: (st) => steps.push(st.step) })
    expect(steps).toEqual(['sending'])
    expect(s).toEqual({ step: 'sent', message: 'Thanks! Report #TVWXYZ sent.' })
  })

  it('unreachable: offers saving', async () => {
    const s = await submitFlow('x', { submit: async () => { throw "can't reach the server: dns error" }, onStep: () => {} })
    expect(s).toEqual({ step: 'failed', message: "Couldn't reach the server." })
  })

  it('refused: shows the server reason', async () => {
    const s = await submitFlow('x', { submit: async () => { throw 'slow down a little, then try again' }, onStep: () => {} })
    expect(s).toEqual({ step: 'failed', message: 'slow down a little, then try again' })
  })

  it('saved: says where and what to do', async () => {
    const s = await saveFlow('x', { save: async () => 'C:\\Users\\sam\\Downloads\\pulse-report-20261012-132507.txt' })
    expect(s.step).toBe('saved')
    expect(s.message).toBe('Saved to C:\\Users\\sam\\Downloads\\pulse-report-20261012-132507.txt — send it to Greg on WhatsApp.')
  })

  it('save failure is reported, not swallowed', async () => {
    const s = await saveFlow('x', { save: async () => { throw 'permission denied' } })
    expect(s).toEqual({ step: 'failed', message: "Couldn't save the report: permission denied" })
  })
})
```

and a stub `client/ui/src/lib/report.ts`:

```ts
export type ReportStep = 'edit' | 'sending' | 'sent' | 'failed' | 'saving' | 'saved'
export type ReportState = { step: ReportStep; message: string }

export async function submitFlow(_text: string, _deps: { submit: (t: string) => Promise<string>; onStep: (s: ReportState) => void }): Promise<ReportState> {
  return { step: 'edit', message: '' }
}

export async function saveFlow(_text: string, _deps: { save: (t: string) => Promise<string> }): Promise<ReportState> {
  return { step: 'edit', message: '' }
}
```

- [ ] **Step 2: Run, expect FAIL** — `cd client/ui && pnpm vitest run src/lib/report.test.ts` → 5 FAIL.

- [ ] **Step 3: Implement** `report.ts`:

```ts
import { errorText } from './tauri'

export type ReportStep = 'edit' | 'sending' | 'sent' | 'failed' | 'saving' | 'saved'
export type ReportState = { step: ReportStep; message: string }

/** The Rust side retries once on a network blip; a failure here is final. */
export async function submitFlow(
  text: string,
  deps: { submit: (t: string) => Promise<string>; onStep: (s: ReportState) => void },
): Promise<ReportState> {
  deps.onStep({ step: 'sending', message: '' })
  try {
    const short = await deps.submit(text)
    return { step: 'sent', message: `Thanks! Report #${short} sent.` }
  } catch (e) {
    const msg = errorText(e)
    return { step: 'failed', message: msg.startsWith("can't reach the server") || msg.startsWith('the server is having trouble') ? "Couldn't reach the server." : msg }
  }
}

export async function saveFlow(text: string, deps: { save: (t: string) => Promise<string> }): Promise<ReportState> {
  try {
    const path = await deps.save(text)
    return { step: 'saved', message: `Saved to ${path} — send it to Greg on WhatsApp.` }
  } catch (e) {
    return { step: 'failed', message: `Couldn't save the report: ${errorText(e)}` }
  }
}
```

- [ ] **Step 4: Run, expect PASS** — `pnpm vitest run src/lib/report.test.ts` → 5 passed.

- [ ] **Step 5: IPC wrappers** — add to `api` in `lib/tauri.ts`:

```ts
  /** Returns the report's short id. */
  submitReport: (text: string) => invoke<string>('submit_report', { text }),
  /** Writes a .txt to Downloads; returns its path. `serverUrl` when not signed in. */
  saveReport: (text: string, serverUrl: string | null) => invoke<string>('save_report', { text, serverUrl }),
```

- [ ] **Step 6: Token** — `app.css` `:root`: add `--warn: #e8b04a;` next to `--danger`.

- [ ] **Step 7: Modal** — `client/ui/src/components/ReportBug.svelte` (same overlay pattern as Settings.svelte):

```svelte
<script lang="ts">
  import Icon from './Icon.svelte'
  import { api } from '../lib/tauri'
  import { saveFlow, submitFlow, type ReportState } from '../lib/report'

  /** `saveOnly`: not signed in (login screen) — the report can only be saved to a file. */
  let { onClose, saveOnly = false, serverUrl = null }: { onClose: () => void; saveOnly?: boolean; serverUrl?: string | null } = $props()

  let text = $state('')
  let st = $state<ReportState>({ step: 'edit', message: '' })
  const busy = $derived(st.step === 'sending' || st.step === 'saving')

  async function send(e: SubmitEvent) {
    e.preventDefault()
    if (saveOnly) return save()
    st = await submitFlow(text, { submit: api.submitReport, onStep: (s) => (st = s) })
    if (st.step === 'sent') setTimeout(onClose, 1500)
  }

  async function save() {
    st = { step: 'saving', message: '' }
    st = await saveFlow(text, { save: (t) => api.saveReport(t, serverUrl) })
  }
</script>

<div class="overlay">
  <div class="backdrop" role="presentation" onclick={onClose}></div>
  <form class="modal" role="dialog" aria-label="Report a bug" onsubmit={send}>
    <div class="top">
      <h1>Report a bug</h1>
      <button type="button" class="close" aria-label="Close" onclick={onClose}><Icon name="x" /></button>
    </div>
    <textarea bind:value={text} maxlength="4000" rows="6" required placeholder="What happened? What were you doing?" disabled={busy || st.step === 'sent' || st.step === 'saved'}></textarea>
    <p class="note">Includes your app version, system info and recent app log.</p>
    {#if st.message}<p class:ok={st.step === 'sent' || st.step === 'saved'} class:err={st.step === 'failed'} role="status">{st.message}</p>{/if}
    <div class="actions">
      {#if st.step === 'failed' && !saveOnly}
        <button type="button" class="primary" onclick={save}>Save report</button>
      {:else if st.step !== 'sent' && st.step !== 'saved'}
        <button class="primary" disabled={busy || !text.trim()}>{busy ? '…' : saveOnly ? 'Save report' : 'Send'}</button>
      {:else}
        <button type="button" class="primary" onclick={onClose}>Done</button>
      {/if}
    </div>
  </form>
</div>

<style>
  .overlay { position: fixed; inset: 0; z-index: 30; display: grid; place-items: center; padding: 16px; }
  .backdrop { position: absolute; inset: 0; background: rgba(10, 11, 13, .6); }
  .modal { position: relative; width: min(460px, 100%); padding: 22px; border-radius: 16px; background: var(--bg-2); border: 1px solid var(--line); display: flex; flex-direction: column; gap: 12px; }
  .top { display: flex; align-items: center; justify-content: space-between; }
  h1 { margin: 0; font-size: 18px; font-weight: 600; }
  .close { width: 32px; height: 32px; border-radius: 50%; border: 0; background: var(--bg-3); color: var(--text-2); display: grid; place-items: center; }
  textarea { resize: vertical; min-height: 120px; padding: 10px 12px; border-radius: 10px; border: 1px solid var(--bg-4); background: var(--bg-1); font: inherit; font-size: 14px; color: var(--text); }
  .note { margin: 0; font-size: 12px; color: var(--text-3); }
  .ok { margin: 0; font-size: 13px; color: var(--ok); word-break: break-all; }
  .err { margin: 0; font-size: 13px; color: var(--warn); }
  .actions { display: flex; justify-content: flex-end; }
  .primary { height: 38px; padding: 0 18px; border: 0; border-radius: 10px; background: var(--warn); color: #1d1606; font-weight: 600; }
  .primary:disabled { opacity: .6; }
</style>
```

- [ ] **Step 8: Shell** — in `Shell.svelte`:
  - import `ReportBug` and `listen` from `@tauri-apps/api/event` (check how Shell/App already listen; reuse the same import),
  - state `let reportOpen = $state(false)` and `let autoReported = $state(false)`,
  - menu item between the two existing ones:

```svelte
            <button role="menuitem" class="warn" onclick={() => { menuOpen = false; reportOpen = true }}>Report a bug</button>
```

  - style: `.menu .warn { color: var(--warn); }`
  - mount: `{#if reportOpen}<ReportBug onClose={() => (reportOpen = false)} />{/if}` next to the Settings mount,
  - crash notice: in an `$effect` (or `onMount`) `const un = listen('pulse://auto-report-sent', () => (autoReported = true)); return () => { un.then((f) => f()) }`, and render near the existing `.conn` banner:

```svelte
  {#if autoReported}
    <div class="auto-report" role="status">Pulse closed unexpectedly last time, so we sent Greg a report. <button aria-label="Dismiss" onclick={() => (autoReported = false)}>×</button></div>
  {/if}
```

  with `.auto-report { padding: 6px 12px; font-size: 13px; color: var(--warn); background: rgba(232,176,74,.12); display: flex; justify-content: space-between; align-items: center; } .auto-report button { border: 0; background: none; color: inherit; font-size: 16px; }`.

  **Note:** the event can fire before Shell mounts (sign-in is followed quickly by the send). If `App.svelte` already holds global listeners, register this one there and pass `autoReported` down, so the notice isn't missed. Check `App.svelte` first and follow its pattern.

- [ ] **Step 9: Login** — in `Login.svelte`: `import ReportBug from '../components/ReportBug.svelte'`, `let reportOpen = $state(false)`; after the card's `.link` button (outside the form, inside `<main>`):

```svelte
  <button type="button" class="report" onclick={() => (reportOpen = true)}>Report a bug</button>
  {#if reportOpen}<ReportBug saveOnly serverUrl={serverUrl} onClose={() => (reportOpen = false)} />{/if}
```

style: `main` becomes `display: grid; place-items: center; align-content: center; gap: 14px;` and `.report { background: none; border: 0; color: var(--warn); font-size: 13px; opacity: .8; } .report:hover { opacity: 1; }`.

- [ ] **Step 10: Checks** — `cd client/ui && pnpm check && pnpm test` → 0 errors, all tests pass.

- [ ] **Step 11: Whole repo** — `just check` (from the repo root) → exit 0.

- [ ] **Step 12: Manual check** — build and run against the dev server: menu → Report a bug → type → Send → "Thanks! Report #…"; `cargo run -q -p pulse-server --bin pulse-app-server -- reports` lists it; `-- report <short> --log` shows meta and log. Stop the server, Send again → "Couldn't reach the server." + Save report → file in `~/Downloads`. Log out → login screen link → Save report → file saved.

- [ ] **Step 13: Commit**

```bash
git add client/ui
git commit -m "feat(ui): Report a bug (account menu + login screen), crash-report notice"
```
