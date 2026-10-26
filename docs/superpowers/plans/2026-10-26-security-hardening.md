# Security hardening: implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** a hardening pass ahead of a corporate security review: patched and continuously checked dependencies,
digest-pinned container images, gateway connection limits, "sign out everywhere else", an off switch for the global
keyboard/mouse hooks, and a `SECURITY.md` that describes the result truthfully.

**Architecture:** no new components. Server changes live in `server/src/limits.rs`, `server/src/gateway/` and
`server/src/auth/`; client changes in `client/src-tauri/src/{hotkeys,keybinds,gateway,commands,api}.rs` and the
Settings components; deploy changes in `deploy/` and the `justfile`; CI in `.github/`.

**Spec:** the owner's brief (no separate spec file). Rulings are recorded in the SDD ledger and the final report.

## Global Constraints

- Public repo: never commit the production server IP, secrets, email addresses, real people's names or references
  to unrelated projects. Test users are `alex` and `sam`. Placeholders use `example.com`.
- Do not deploy, ssh to production, dispatch workflows, merge to `main` or change GitHub repository settings.
- TDD: a failing test first for every behaviour change (server, client core, UI). Windows-only code must pass
  `XWIN_ACCEPT_LICENSE=1 cargo xwin clippy -p pulse-client --all-targets --target x86_64-pc-windows-msvc -- -D warnings`
  (`cargo-xwin` lives in `~/.cargo/bin`, which may not be on `PATH`).
- `just check` stays green after every task. Protocol type changes regenerate bindings (`just gen-types`) and commit
  them.
- Match the surrounding code's style: short doc comments that say why, no new dependencies unless needed.
- Defaults keep today's behaviour for users: keybinds ON, no change for a normal number of devices.

## Task 1: Patch dompurify and undici

**Files:** `client/ui/package.json`, `client/ui/pnpm-lock.yaml`.

- [ ] dompurify `>= 3.4.17` (currently 3.4.16, direct dependency).
- [ ] undici `>= 8.12.2` (currently 8.11.2, transitive via jsdom → also vitest). If jsdom's range does not allow
  8.12.2, add a `pnpm.overrides` entry (`"undici": "^8.12.2"` or the narrowest equivalent) and say why in the report.
- [ ] `pnpm why dompurify undici` shows only patched versions; `pnpm audit` shows no high/critical; `pnpm check` and
  `pnpm test` green.

## Task 2: Dependency checks in CI (cargo deny, pnpm audit, Dependabot)

**Files:** `deny.toml` (new, repo root), `.github/workflows/ci.yml`, `.github/dependabot.yml` (new), possibly
`Cargo.toml`/`Cargo.lock`, `justfile` (optional `deny` recipe), `THIRD_PARTY.md` if licences need noting.

- [ ] Install `cargo-deny` locally (`cargo install cargo-deny --locked`) and write `deny.toml` covering
  `advisories`, `licenses`, `bans`, `sources`. Tune it so `cargo deny check` passes on this branch:
  - licences: an explicit allow-list of what the tree actually uses (no blanket allow); per-crate `exceptions` for
    oddballs; `vendor/webrtc-sys` (a path crate patched in via `[patch]`) handled explicitly (e.g. private/unpublished
    crates ignored for licence checks, or a clarify/exception entry), with a comment explaining it.
  - sources: crates.io only, plus any git sources actually in use, each listed explicitly.
  - bans: `multiple-versions = "warn"` (duplicates are normal in a Tauri tree); no wildcard path deps flagged
    as errors for workspace members.
  - advisories: fix what can be fixed by `cargo update -p` (minimal, compatible bumps; rerun the full tests). Anything
    that cannot be fixed gets an `ignore` entry with the advisory id and a one-line justification (why it does not
    affect Pulse or why no fix exists).
- [ ] CI (`ci.yml`): a `deps` paths filter (`Cargo.toml`, `Cargo.lock`, `**/Cargo.toml`, `deny.toml`,
  `client/ui/package.json`, `client/ui/pnpm-lock.yaml`, `.github/workflows/ci.yml`) and two jobs:
  - `cargo-deny`: `EmbarkStudios/cargo-deny-action` (pinned major) running `check` with all four checks.
  - `pnpm-audit`: in `client/ui`, `pnpm audit --prod --audit-level high`.
  - Both also run on a weekly `schedule` trigger (new advisories appear without code changes); other jobs must not
    run on the schedule. Keep `permissions: contents: read`.
- [ ] `.github/dependabot.yml`: weekly updates for `cargo` (/), `npm` (/client/ui), `github-actions` (/), and Docker
  images: `docker` for the Dockerfiles in use and `docker-compose` for `/deploy` if Dependabot supports it (check
  current docs; if not supported, say so and cover the Dockerfiles only). Group updates to keep PR noise low (e.g. one
  group for minor+patch per ecosystem; security updates still separate). Sensible `open-pull-requests-limit`.
- [ ] Verify: `cargo deny check` exits 0 locally; YAML parses (`python -c 'import yaml…'` or `actionlint` if present).

## Task 3: Pin container images by digest

**Files:** `deploy/compose.yml`, `deploy/deploy.sh`, `deploy/env.example`, `deploy/README.md`, `justfile`
(`deploy-check`, `deploy`), `.github/workflows/ci.yml` (deploy job's Caddy image).

- [ ] Look up the multi-arch **index** digests (not a single-platform manifest) for `livekit/livekit-server:v1.13.10`
  and the newest `caddy:2.10.x` (exact tag, e.g. `2.10.2`). `docker buildx` is not installed: use
  `docker manifest inspect -v` or the registry HTTP API with an anonymous pull token (HEAD on the manifest with the
  OCI index + Docker manifest-list `Accept` headers, read `Docker-Content-Digest`). Confirm each index lists
  `linux/arm64` (prod is ARM64). Record digests and the arm64 check in the report.
- [ ] `compose.yml`: `image: livekit/livekit-server:v1.13.10@sha256:…`, `image: caddy:2.10.x@sha256:…`.
- [ ] pulse-server: `compose.yml` reads `image: ${PULSE_IMAGE:-ghcr.io/greghofmeyr/pulse-server:latest}` (or
  equivalent). `deploy.sh TAG` keeps its current contract and output, plus:
  1. pull `ghcr.io/greghofmeyr/pulse-server:TAG` (a bad tag still fails before anything changes);
  2. resolve the pulled image's repo digest (`docker image inspect --format '{{index .RepoDigests 0}}'` or the
     registry), and refuse to continue if none is found;
  3. write `PULSE_IMAGE=ghcr.io/greghofmeyr/pulse-server:TAG@sha256:…` into `.env` (replace the line if present,
     append if not, so existing servers' `.env` files keep working) and keep `PULSE_TAG=TAG` for humans;
  4. print the pinned reference; the final "healthy" line still prints, with the digest.
  `just deploy <sha7>` UX stays identical. Do not run it.
- [ ] `env.example` documents `PULSE_IMAGE` (written by deploy.sh). `deploy-check` / CI use the pinned Caddy image
  for `caddy validate`, and the CI compose check still passes with the new variable.
- [ ] `deploy/README.md`: how pinning works, how to bump livekit/caddy (look up the index digest, check arm64), and
  that Dependabot proposes bumps.
- [ ] `shellcheck` clean (`just deploy-check` runs it). Add a small test for the `.env` rewrite if it is factored into
  a function (e.g. a bats-free bash self-test is not required; a shellcheck-clean, readable script is).

## Task 4: Gateway connection limits

**Files:** `server/src/limits.rs`, `server/src/testing.rs`, `server/src/gateway/{socket,hub}.rs`, server tests,
`client/src-tauri/src/gateway.rs` (close-code handling), protocol constant if close codes are shared.

- [ ] `LimitsConfig::gateway_ip: Rate` (per client IP, via `ClientIp`): one token spent per `/gateway` upgrade
  request before upgrading; refused with HTTP 429 when empty. Refund the token once the socket authenticates
  successfully, so only failed/abandoned attempts (bad token, no Hello, timeouts) accumulate — same "only failures
  keep the spend" model as login. Default `Rate::per_minute(20)` (burst 20).
- [ ] `LimitsConfig::gateway_per_user: usize` = **8** concurrent authenticated gateway connections per user. The 9th
  is refused after auth with a new close code **4006** (`CLOSE_TOO_MANY_CONNECTIONS`, reason "too many
  connections"); existing connections are never kicked. The check and the registration must be atomic in `Hub`
  (no race where two simultaneous connects both get slot 8).
- [ ] `LimitsConfig::unlimited()` (used by `testing.rs`) leaves both effectively unlimited.
- [ ] Client (`client/src-tauri/src/gateway.rs`): 4006 and an HTTP 429 at upgrade are NOT logout and NOT
  update-required: treat them as a retryable failure with the normal backoff. Test it in the existing gateway tests'
  style.
- [ ] Server tests: per-IP refusal with 429 after N failed auths; successful auths don't use up the bucket; 9th
  connection closed with 4006 while the first 8 stay up; a slot frees when one disconnects.

## Task 5: Sign out everywhere else

**Files:** `server/src/auth/{routes,session}.rs`, `server/src/gateway/hub.rs`, `protocol/` (response type +
bindings), `client/src-tauri/src/{api,commands,lib}.rs`, `client/ui/src/lib/tauri.ts`, Settings UI component(s) +
tests.

- [ ] Endpoint `POST /auth/logout-others` (bearer auth): deletes every session of the caller except the one making
  the request; closes the caller's live gateway sockets that belong to the revoked sessions with 4001 (so those
  devices drop to the login screen); returns `{ "revoked": <n> }` (new protocol type, ts bindings generated). The
  current device stays signed in. (Signing out this device already exists: `POST /auth/logout`.)
- [ ] `Hub` gets a method closing a user's connections whose session is not `keep_hash`.
- [ ] Server tests: two/three sessions for `alex` + one for `sam`; after the call only alex's caller session
  authenticates, sam's is untouched, alex's other gateway socket gets 4001, the caller's socket stays open; the count
  is right; unauthenticated call is 401.
- [ ] Client core: `api.logout_others(token)` + Tauri command `logout_others` registered in `lib.rs`; returns the
  count.
- [ ] UI: in Settings → Advanced, a "Sessions"/"Account" block with a "Sign out all other devices" button; clicking
  asks for confirmation ("are you sure" — in-app confirm, matching existing patterns); on confirm calls the command and
  shows the result ("Signed out N other devices") or an error. Vitest tests: confirm → command called; cancel → not
  called; result/error shown.

## Task 6: Global keybinds master switch

**Files:** `client/src-tauri/src/{keybinds,hotkeys,lib}.rs`, `client/ui/src/lib/{keybinds,tauri}.ts`,
`client/ui/src/components/KeybindSettings.svelte` + tests.

- [ ] A persisted `enabled` flag (default **true**, missing field = true so existing `keybinds.json` files keep
  working) stored alongside the binds.
- [ ] Settings → Keybinds: a master "Global keybinds" toggle at the top with a one-line hint (Pulse watches
  keyboard/mouse system-wide only to match the binds; off = no hooks at all). Off disables the bind editors.
- [ ] Windows (`hotkeys.rs`): off ⇒ both `WH_KEYBOARD_LL` and `WH_MOUSE_LL` hooks are unhooked (on the hook thread,
  which owns them), and the watchdog does not reinstall them while off; on ⇒ they are installed again. At startup with
  the switch off the hooks are never installed. The decision logic must be unit-testable off Windows (e.g. a small
  pure state type or by gating the watchdog on an `AtomicBool`, with tests of the pure part).
- [ ] Linux: the flag is stored and shown; the compositor/CLI path (ipc.rs) is unaffected (say so in the hint only if
  the UI already distinguishes platforms).
- [ ] Tests: Rust (load/save default + round-trip, the enable/disable decision logic), UI (toggle calls the command,
  persists, disables the editors). xwin clippy clean.

## Task 7: SECURITY.md

**Files:** `SECURITY.md` (repo root), link from `README.md`.

- [ ] Written for a corporate security reviewer; every claim checked against the code/config on this branch (cite the
  file in your report, not in the document). Sections:
  1. What the client accesses and why: global keyboard/mouse low-level hooks (Windows; matched against saved binds
     only, never stored or sent, can be switched off — Task 6); screen/window capture, process audio loopback and
     process listing only on explicit Go Live; microphone for voice.
  2. Network: outbound only — HTTPS/WSS to the server domain, LiveKit media UDP 7882 / TCP 7881 / TURN 3478 (check
     `deploy/livekit.yaml` and the Caddyfile for the real ports); the client opens no listening ports (verify: check
     `ipc.rs` — if it uses a local socket/named pipe, describe it accurately); SFU, so no peer-to-peer media to other
     users.
  3. Local data: token in the OS credential store (Windows Credential Manager); logs never contain tokens (verify the
     redaction); media disk cache (where, what); what bug/crash reports send.
  4. Server-side protections: argon2 passwords, rate limits (list them, incl. Task 4), 256-bit opaque tokens stored as
     SHA-256 with 90-day sliding expiry and revocation (incl. Task 5), media login-gated and re-encoded server-side with
     decode budgets, sanitised markdown, strict CSP (quote the real policy's shape), minimal Tauri capabilities.
  5. Infrastructure (no IP, no hostnames beyond the public product domain if needed): TLS via Caddy; only listed ports
     public; SSH not public; containers non-root; images pinned by digest (Task 3); encrypted off-box backups; LiveKit
     TURN refuses loopback/private/link-local peers.
  6. Build and supply chain: CI-built release artifacts; no secrets beyond `GITHUB_TOKEN` (check the release
     workflows); cargo deny, pnpm audit, Dependabot (Task 2).
  7. Known gaps: installer not code-signed yet; no auto-updater (manual installs); anything else found untrue while
     verifying.
  8. Reporting an issue: GitHub private vulnerability reporting / a security advisory on the repo (no email address).
- [ ] Nothing claimed that is not true; anything that turned out false is either fixed (if trivially in scope) or
  listed as a known gap.
