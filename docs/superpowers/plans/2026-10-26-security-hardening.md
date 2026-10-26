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

---

# Wave 2: no leftovers

The first wave's review left follow-ups and known gaps. The owner's rule is "leave no leftovers", so this wave closes
them. Same Global Constraints as above. Remaining deliberate known gaps (SECURITY.md must state them clearly): no
end-to-end encryption; no MFA (owner decides separately); the deploy service user is in the `docker` group (mitigation:
only the owner's SSH keys over Tailscale reach it).

## Task 8: Clear client message for a refused connection (4006 / 429)

**Files:** `client/src-tauri/src/gateway.rs` (connection state / Boot), the UI that renders connection state, tests.

- [ ] A 4006 close shows "Too many devices connected — close Pulse on another device" (or equivalent wording matching the
  app's tone) instead of "server unreachable"/plain "Reconnecting"; the client keeps retrying with backoff.
- [ ] An HTTP 429 at upgrade shows "Too many attempts, retrying in Ns" with the real backoff delay.
- [ ] Neither is LoggedOut/UpdateRequired. Tests in gateway.rs's existing style (state emitted) and a UI test for the
  rendering.

## Task 9: Re-encode Go Live preview frames

**Files:** `server/src/streams/routes.rs` (put_preview/get_preview), `server/src/media/` (reuse its image pipeline and
decode budgets), tests.

- [ ] `put_preview` decodes the uploaded image under the media pipeline's decode budget, re-encodes it as WebP with
  metadata stripped and fixed maximum dimensions (480×270, aspect kept, never upscaled), and stores/serves only the
  re-encoded bytes. Undecodable or over-budget input → 4xx, nothing stored. Keep the 100 KB input cap.
- [ ] Tests: a valid preview round-trips as WebP within 480×270; garbage/non-image/oversized-dimension input is rejected;
  served bytes differ from uploaded bytes when the upload carried metadata (or is a different format).

## Task 10: Revocation removes LiveKit participants; live-only revoked count

**Files:** `server/src/auth/{routes,session}.rs`, `server/src/voice/`, `server/src/streams/`, LiveKit API client code
already used by `voice/restore.rs`, tests.

- [ ] On `/auth/logout` and `/auth/logout-others`, every LiveKit participant (voice room and stream rooms) that was
  admitted with a token issued to a revoked session is removed via the LiveKit RoomService `RemoveParticipant`.
  Participants admitted through sessions that stay valid are untouched (e.g. the caller's own voice on logout-others).
  Voice identity is `<user ulid>`, stream identities `<user ulid>#v` / `#a` — the server must remember which session
  each issued token belongs to (server-side only; session hashes never go into LiveKit tokens/metadata).
- [ ] Failure to reach LiveKit must not fail the logout (log it; revocation of REST/gateway access already happened).
- [ ] `revoked` counts only sessions that were live (unexpired); expired rows are still deleted.
- [ ] Tests: a mock or the existing LiveKit test harness verifying RemoveParticipant calls for exactly the revoked
  sessions' participants; count excludes expired.

## Task 11: No binds = no hooks; hook-thread edge cases

**Files:** `client/src-tauri/src/{hotkeys,hookwatch,keybinds}.rs`, tests.

- [ ] Hooks are installed only when the switch is on AND at least one bind exists: adding the first bind installs, removing
  the last uninstalls, the watchdog's gate treats "no binds" like "off". Startup with no binds installs nothing.
  Recording a new bind while none exist must still work (decide how — e.g. recording happens in the webview, not via the
  hooks; verify).
- [ ] If the hook thread never started, `set_enabled(true)` (and the new bind-driven path) must not report "on, no
  problem" — surface the failure.
- [ ] The reinstall retry loop stops promptly when the switch goes off (or binds become empty): no input-API polling
  while off, and `HOOKED` reflects reality.
- [ ] Pure decision logic unit-tested; xwin clippy clean.

## Task 12: Owner-only session.json fallback

**Files:** `client/src-tauri/src/session.rs`, tests.

- [ ] Unix: 0600 (already) — keep the test. Windows: the file's DACL grants access only to the current user (protected
  DACL, inheritance off), set atomically at creation (no window where it's readable with inherited ACLs).
- [ ] Test what's testable: a Windows-only test that reads back the DACL (compiles under xwin; runs on Windows CI if any)
  and the Unix mode test.

## Task 13: "Send crash reports automatically" toggle

**Files:** `client/src-tauri/src/reports/`, settings storage + command, the Settings UI (Advanced), tests.

- [ ] Toggle "Send crash reports automatically", default on, persisted. Off: a crash report is saved locally and on next
  launch the user is offered "Send" / "Don't send" (the saved report is deleted either way after the choice). Manual bug
  reports unchanged.
- [ ] Tests: Rust (default on, off ⇒ not auto-sent and kept pending; choice paths), UI (toggle, the next-launch prompt).

## Task 14: No third-party STUN handed to clients

**Files:** `deploy/livekit.yaml`, `docker/livekit.yaml` (dev), deploy/README.md if relevant.

- [ ] Find in LiveKit v1.13.10 source how ICE servers handed to clients are built and how STUN is used for external IP
  discovery (`use_external_ip`). Configure prod (and dev, to test it) so clients are not given third-party STUN servers
  (node's own TURN/STUN only, or none) while the server can still determine its public IP without a third party if
  possible — without committing the server IP. If an unavoidable third-party lookup remains (e.g. server-side IP
  discovery), say exactly what and where.
- [ ] `just voice-it` passes with the dev config change (coordinate with any other worktree's LiveKit; never stop other
  sessions' containers).

## Task 15: Verify the libwebrtc download

**Files:** `vendor/webrtc-sys/build.rs` (and whatever it needs), docs.

- [ ] Pin the SHA-256 of the prebuilt libwebrtc archive for every target we build (linux x64 — CI + dev; windows msvc x64
  — release; any other target a workflow builds) and verify it before extraction; mismatch fails the build with a clear
  message. `LK_CUSTOM_WEBRTC` (local build) bypasses the download and the check, as now. Document how to bump.
- [ ] Verify: a clean download path computes and checks the hash (e.g. remove the cached archive/dir in a scratch
  `CARGO_TARGET_DIR` and build `webrtc-sys`), and a deliberately wrong hash fails.

## Task 16: Deterministic login rate-limit test

**Files:** `client/src-tauri/src/api.rs` tests (and whatever the root cause touches).

- [ ] Find why `api::tests::too_many_failed_logins_says_slow_down` failed under load (systematic debugging; evidence),
  fix the root cause so it is deterministic. Prove it with a stress loop (e.g. 50 runs while the machine is busy).

## Task 17: SECURITY.md wave-2 update

**Files:** `SECURITY.md`.

- [ ] TURN: LiveKit v1.13.10's embedded TURN refuses restricted peers — verify against the tag
  (`gh api 'repos/livekit/livekit/contents/pkg/service/turnrelay.go?ref=v1.13.10'` and `pkg/service/turn.go`):
  `isRestrictedPeerIP` (loopback, link-local unicast/multicast, multicast, private, unspecified) applied in the TURN
  PermissionHandler, `newTURNRelayPolicy` limiting relays to the node's own IPs + media ports, TCP relay rejected,
  per-user allocation quota; our livekit.yaml sets no allow-list. State it with that source reference.
- [ ] Reflect Tasks 8–16 (remove the closed gaps; describe the new behaviour precisely).
- [ ] Known gaps left: no E2E encryption; no MFA; deploy user in the docker group (mitigation: only the owner's SSH keys
  over Tailscale reach it). Every claim re-verified against the final code.

## Task 18: Pin the remaining build inputs

The wave-1 SECURITY.md listed floating build inputs as a known gap; under the "no leftovers" rule they get pinned.

**Files:** `.github/workflows/*.yml`, `deploy/server.Dockerfile`, `rust-toolchain.toml`, `.github/dependabot.yml`,
`deploy/README.md`/`README.md` (how to bump).

- [ ] Every third-party GitHub Action pinned to a full commit SHA with a `# vX.Y.Z` comment (Dependabot's
  github-actions updater keeps them current).
- [ ] `server.Dockerfile` base images pinned `tag@sha256:<multi-arch index digest>` (check linux/amd64 + linux/arm64 are
  in the index; Dependabot's docker updater bumps digests).
- [ ] The Tauri CLI used by release-windows.yml pinned to an exact version (matching the `tauri` crate's minor), not
  `^2`.
- [ ] The Rust toolchain pinned to an exact version in `rust-toolchain.toml`, used by CI and release workflows (no
  `@stable` drift); document how to bump.
- [ ] ci.yml's LLVM install no longer pipes an unverified script to bash: fetch `llvm.sh` at a pinned commit and verify
  its SHA-256 (or install from the signed apt repo directly), failing on mismatch.
- [ ] Dependabot config still covers what's pinned; YAML valid; `just check` + `just deploy-check` green.
