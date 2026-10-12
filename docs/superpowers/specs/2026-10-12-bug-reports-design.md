# Bug reports — design

Date: 2026-10-12. Status: draft for review.

## Why

Friends run the Windows release on networks and machines we can't see. When something goes wrong they
should be able to say so in one click, and the report should carry what we need to find the cause:
which build, which Windows, what the connection and voice path were doing, and the recent app log.
Over time this is also the real-world data (networks, locations, devices) we tune voice against.

## What the friend sees

- **Signed in:** the account menu (Shell.svelte, next to "Voice & audio settings" / "Log out") gets an
  orange **Report a bug** item. It opens a modal (the existing inline overlay pattern).
- **Login screen:** a small **Report a bug** link under the card. Same modal, but there is no session,
  so it only offers **Save report** (see Sending).
- **The modal:** a heading, a textarea ("What happened? What were you doing?", required, up to
  4000 characters), the line *"Includes your app version, system info and recent app log."* and
  **Send**. On success: "Thanks! Report #&lt;short id&gt; sent." and the modal closes.

No preview of the attached data, no opt-out: friends have agreed to it, and nothing sensitive is in it
(checked: no log line contains a token, password or message text).

## What a report contains

Gathered by the client in Rust (one new command), so the webview never touches files:

| Field | Source |
|---|---|
| text | the textarea |
| app version + commit | `CARGO_PKG_VERSION` + `option_env!("PULSE_COMMIT")` (set by the release workflows from `GITHUB_SHA`; `dev` otherwise) |
| OS | `os_info` crate: e.g. "Windows 11 Home 10.0.26100 (64-bit)" |
| server URL | the session's `api.base()` (or what's typed on the login screen) |
| gateway state | new `Mutex<ConnState>` in `Core`, set in `forward()` |
| voice | current channel (if any), mute/deafen controls, configured input/output device names |
| log tail | last 256 KB of the newest file in the profile's log dir |

Everything except text and log goes into one `meta` JSON object, so adding a field later needs no
migration. The voice path numbers (rtt / jitter / buffer / loss, logged every 5 s in a call) and the new
request-failure lines arrive via the log tail.

## Sending

1. Signed in: `POST /reports` with the report (JSON). On a network error or 5xx, wait 2 s and retry
   **once**.
2. If that still fails, or on the login screen: the modal shows "Couldn't reach the server." and a
   **Save report** button. Rust writes `pulse-report-<YYYYMMDD-HHMMSS>.txt` (human-readable: the
   fields, then the log) to the user's Downloads folder (fallback: Desktop, then the app data dir) and
   the modal shows the full path: "Saved to … — send it to Greg on WhatsApp."

No Tauri dialog/fs plugin needed: Rust picks the path itself.

## Server

- Migration `0003_bug_reports.sql`:
  `bug_reports(id TEXT PK, user_id TEXT NOT NULL REFERENCES users, created_at TEXT NOT NULL,
  text TEXT NOT NULL, meta TEXT NOT NULL, log TEXT NOT NULL, seen_at TEXT NULL)`.
- `POST /reports` (AuthUser): validates text 1–4000 chars, meta ≤ 16 KB, log ≤ 512 KB; returns
  `{ id }`. Rate limit `report_user = Rate::per_hour(5)` (new `LimitsConfig` field). The route gets
  `DefaultBodyLimit::max(1 MB)`; everything else keeps axum's 2 MB default.
- Protocol: `CreateReportRequest { text, meta: serde_json::Value, log }` and `CreateReportResponse { id }`
  in `protocol/src/rest.rs`, exported to TS like the others.
- Reports are write-only over HTTP. Nobody, Greg included, reads them through the API.

## Reading reports (Greg)

New server subcommands, wrapped by `just` recipes over Tailscale SSH (same pattern as `invite-prod`):

- `just reports` → `pulse-app-server reports`: newest 20, one line each — short id, time, username,
  `new` if unseen, first 60 chars of the text.
- `just report <id>` → `pulse-app-server report <id>`: everything except the log, pretty-printed meta;
  marks it seen.
- `just report <id> --log` → also prints the log tail.

`<id>` accepts a unique prefix (the short id shown in lists).

## Log hygiene (part of this work)

Found while designing this: one broken ALSA device wrote ~200,000 identical warnings and a 2.2 GB log
in a morning. On a friend's PC that fills their disk, and it would make the log tail useless.

1. **Repeated errors collapse.** The cpal stream-error callbacks (devices.rs, input and output, F32 and
   I16) go through a small throttle: the first occurrence logs, repeats are counted, and at most one
   "…repeated N times" line is written per 10 s.
2. **Daily size cap.** The log writer stops at 50 MB per file and writes one "log size cap reached"
   line. (Rolling stays daily, 7 files kept.)
3. **Failed requests are logged.** `Api` logs one line per failed request: method, path (no query, no
   body), error kind, elapsed ms. Successful requests stay silent. Never the token or body.

## Out of scope

Screenshots, attaching files, a reply channel to the friend, reading reports anywhere but the CLI, any
notification when a report arrives, crash auto-reporting.

## Testing

- Server: create report (happy path, id returned, row stored); text empty / too long → 400; oversize
  log → 400 or 413; no auth → 401; sixth report in an hour → 429; `reports`/`report` CLI functions
  (list order, prefix lookup, ambiguous prefix error, marks seen).
- Client Rust: log-tail reads only the last N bytes of the newest file (and handles no file); report
  text formatting for the saved file; retry-once policy (network error → one retry → give up; 4xx → no
  retry); throttle (first logs, repeats counted, summary once per window); size-capped writer stops at
  the cap and writes the notice once; request-failure log line contains no token.
- UI: the modal's states (idle → sending → sent / failed → saved) with the Tauri calls mocked, as the
  existing UI tests do.
