# Two-factor authentication (TOTP): implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** optional authenticator-app 2FA for Pulse accounts: enrol from Settings → Advanced, a code step at login,
single-use recovery codes, an owner reset command, TOTP secrets encrypted at rest with a new server key.

**Architecture:** a new `server/src/auth/totp.rs` (key, AEAD, codes) and `server/src/auth/two_factor.rs` (routes);
the login route gains a ticket step. Client: `api.rs` + `commands.rs` hold the ticket and render the QR matrix;
Svelte gets a login code step and a `TwoFactorSettings` component. Deploy gets `PULSE_TOTP_KEY`.

**Spec:** `docs/superpowers/specs/2026-10-26-two-factor-design.md` (binding). Its §2 table holds every exact value.

## Global Constraints

- Public repo: never commit IPs, secrets, email addresses, real people's names, or references to other projects.
  Test users are `alex` and `sam`. Placeholders use `example.com`.
- Do not deploy, ssh anywhere, dispatch workflows, merge to `main`, or change GitHub settings. Never bare
  `git stash`. Work only in this worktree; don't touch other worktrees or their
  Docker containers.
- TDD: a failing test first for every behaviour change (server, client core, UI). `just check` stays green after
  every task. Protocol changes: `just gen-types` and commit the regenerated `client/ui/src/lib/protocol` files.
- Never log or put in an error message: TOTP secrets, codes, recovery codes, tickets, passwords. No outbound calls.
- Errors to a **signed-in** caller never use 401 (the client treats 401 outside login as a dead session). Wrong
  password/code there → `AppError::ForbiddenMsg`.
- Exact values (spec §2): SHA-1, 6 digits, 30 s, ±1 step, last-used step remembered; 20-byte secrets, base32 (RFC
  4648, no padding); otpauth URI `otpauth://totp/Pulse:<username>?secret=<B32>&issuer=Pulse&algorithm=SHA1&digits=6&period=30`;
  `PULSE_TOTP_KEY` = exactly 64 hex chars; ChaCha20-Poly1305, 12-byte random nonce, BLOB `nonce ‖ ciphertext+tag`,
  AAD `pulse-totp-v1:<user id>`; 10 recovery codes of 16 chars from `abcdefghjkmnpqrstuvwxyz23456789`, shown
  `xxxx-xxxx-xxxx-xxxx`, normalised by removing spaces and `-` and lowercasing, stored as SHA-256 hex; tickets 256-bit
  random base64url, SHA-256 stored, 5-minute expiry, deleted on success or after 5 wrong codes; limits `mfa_ip` =
  `Rate::per_minute(5)`, `mfa_user` = `Rate::per_hour(10)` (keyed by user id), spent up front, refunded on success.
- A code is TOTP when, spaces removed, it is exactly 6 ASCII digits; otherwise it's tried as a recovery code where
  recovery codes are allowed (login, disable), and is simply wrong where they aren't (enable, regenerate).
- Match the surrounding style: short doc comments that say why; no new dependencies beyond `totp-rs`,
  `chacha20poly1305` (server) and `qrcode` (client), each with `default-features = false` plus only what is needed.
  `cargo deny check` must pass.

## Task 1: TOTP core, key, migration

**Files:** `server/Cargo.toml`, `Cargo.lock`, `server/migrations/0007_two_factor.sql` (new),
`server/src/auth/totp.rs` (new), `server/src/auth/mod.rs`, `server/src/config.rs`, `server/src/testing.rs`.

- [ ] Dependencies: `totp-rs = { version = "6", default-features = false, features = ["std"] }` (or the minimal
  feature set that gives `TOTP::generate`), `chacha20poly1305 = { version = "0.10"|"0.11", default-features = false,
  features = [...] }` (pick the version that resolves cleanly with the workspace's `rand`; use `rand::rng()` for
  nonces and secrets as the rest of the server does).
- [ ] Migration `0007_two_factor.sql`: tables `user_totp`, `recovery_codes`, `mfa_tickets` exactly as spec §3
  "Storage" (FKs `ON DELETE CASCADE`, `STRICT` only if the other migrations use it).
- [ ] `auth/totp.rs`:
  - `TotpKey` (32 bytes, no `Debug` that prints bytes); `TotpKey::parse(&str) -> anyhow::Result<TotpKey>` accepting
    exactly 64 hex chars (either case); errors never echo the input.
  - `new_secret() -> [u8; 20]`; `secret_base32(&[u8]) -> String`; `otpauth_uri(username, &[u8]) -> String`.
  - `encrypt(key, user: UserId, secret) -> Vec<u8>` / `decrypt(key, user, blob) -> anyhow::Result<Vec<u8>>`
    (AAD as above; a wrong key, wrong user or tampered blob fails).
  - `match_step(secret, code, now_unix) -> Option<i64>`: the step (unix/30) in {t-1, t, t+1} whose code equals
    `code`, constant-time; `None` for anything that isn't 6 digits.
  - `is_totp_shaped(code) -> bool` (6 digits after removing spaces).
  - Recovery: `new_recovery_codes() -> Vec<String>` (10, formatted), `normalize_recovery(&str) -> String`,
    `hash_recovery(&str) -> String` (hash of the normalised form).
- [ ] `Config.totp_key: Option<TotpKey>`: `PULSE_TOTP_KEY` unset or empty → `None`; set but malformed → `from_env`
  fails with a message naming the variable but not its value. `testing::test_config()` uses a fixed test key.
- [ ] Unit tests (in `totp.rs`): RFC 6238 SHA-1 test vectors (8-digit vectors truncated to 6 is NOT valid: use the
  RFC secret `12345678901234567890` and compute expected 6-digit codes via `totp-rs` itself at a fixed time, plus at
  least one known 6-digit value, e.g. t=59 → `287082`); ±1 step accepted, ±2 refused, step returned; non-digit /
  5- / 7-digit codes refused; encrypt/decrypt round trip, wrong key fails, other user's AAD fails, tampered byte
  fails, two encryptions of one secret differ (random nonce); key parse accepts 64 hex, refuses 63/65/non-hex and
  the error doesn't contain the input; recovery codes: 10, distinct, format regex `^[a-hjkmnp-z2-9]{4}(-[a-hjkmnp-z2-9]{4}){3}$`,
  normalise(" ABCD-efgh jkmn-PQRS ") == "abcdefghjkmnpqrs", hash is of the normalised form; otpauth URI exact string
  for `alex`.

## Task 2: Enrolment, status, off, regenerate (server)

**Files:** `protocol/src/rest.rs`, `client/ui/src/lib/protocol/*` (generated), `server/src/limits.rs`,
`server/src/auth/two_factor.rs` (new), `server/src/auth/mod.rs`, `server/src/auth/routes.rs` (only if sharing a
helper, e.g. logout-others' revoke sequence), `server/src/lib.rs`, `server/tests/two_factor.rs` (new).

- [ ] Protocol (TS-exported like the neighbours): `TwoFactorStatus { enabled: bool, available: bool,
  recovery_codes_left: u32 }`, `TwoFactorSetupRequest { password }`, `TwoFactorSetupResponse { secret, otpauth_uri }`,
  `TwoFactorCodeRequest { code }`, `TwoFactorEnabledResponse { recovery_codes: Vec<String>, signed_out: u32 }`,
  `RecoveryCodesResponse { recovery_codes: Vec<String> }`.
- [ ] Limits: `mfa_ip` and `mfa_user` in `LimitsConfig` (defaults per Global Constraints, `unlimited()` too) and
  `Limits`. A helper used by every 2FA check (here and in Task 3): spend both up front; if either is out, refund the
  other and answer 429; refund both on success.
- [ ] Routes (spec §3 "HTTP" table): `GET /auth/2fa`, `POST /auth/2fa/setup`, `/enable`, `/disable`,
  `/recovery-codes`. Setup verifies the password with `password::verify_async` and stores a pending encrypted secret
  (replacing any pending one). Enable verifies the code against the pending secret, sets `enabled_at` and
  `last_step` (the matched step), writes 10 recovery code hashes, then signs out the caller's other sessions with the
  same three steps as `logout_others` (`session::revoke_others`, `hub.drop_other_sessions`,
  `voice::revoke::remove_revoked`); `signed_out` = live sessions revoked. Use `db::begin_write` where a read decides
  a write. Disable accepts TOTP or recovery code; deletes `user_totp`, recovery codes and tickets. Regenerate accepts
  TOTP only and replaces all codes. TOTP acceptance always goes through the atomic `last_step` update.
- [ ] Tests (`server/tests/two_factor.rs`, compute codes with `totp-rs` from the setup's secret at "now", and pick
  steps explicitly where reuse matters): status off/available; setup wrong password 403 (and session survives: a
  following `/me` is 200); setup → enable with a wrong code 403 → with the right code 200 + 10 codes in the format;
  status on with 10 left; the enabling session stays signed in and another session of the same user is signed out
  (its `/me` → 401) and `signed_out == 1`; another user's session untouched; setup while on → 409; enable without
  setup → 409; secret stored is not the plaintext (the BLOB doesn't contain the 20 raw bytes) and recovery codes are
  stored only as hashes; regenerate with TOTP → 10 new codes, old ones no longer work, with a recovery code → 403;
  disable with a recovery code works once (second use of the same code fails) and with TOTP works; the same TOTP code
  used twice (enable then regenerate within the step) → second refused; disable when off → 409; rate limit: with
  tight `mfa_user`, wrong codes hit 429 and a refused attempt costs nothing; no key (`totp_key: None`) → status
  `available: false` and setup 403.

## Task 3: Login code step (server)

**Files:** `protocol/src/rest.rs`, generated TS, `server/src/auth/routes.rs`, `server/src/auth/two_factor.rs`,
`server/tests/two_factor_login.rs` (new).

- [ ] Protocol: `MfaRequired { code: String /* "mfa_required" */, message: String, ticket: String }` and
  `LoginMfaRequest { ticket, code }`.
- [ ] `check_login`: password right and `user_totp.enabled_at` set → prune expired tickets, insert a new ticket,
  answer **403** with `MfaRequired` (message: "Enter the 6-digit code from your authenticator app. If Pulse doesn't
  ask for one, update it."). Login rate-limit tokens are refunded (not a failed password). Users without 2FA: no
  change at all.
- [ ] `POST /auth/login/2fa`: limits helper keyed by the ticket's user and the IP (an unknown ticket spends only the
  IP bucket); unknown/expired/≥5-failure ticket → 410; TOTP (via atomic `last_step`) or recovery (atomic delete) →
  delete the ticket (must affect one row, else 410) → `session::create` → `SessionResponse` (with `avatar_hash` like
  login). Wrong code → increment `failures` (delete at 5) → 401.
- [ ] Tests: no-2FA login unchanged; 2FA user's login → 403 body with `code == "mfa_required"`, a ticket, and no
  session row created; right TOTP → 200 session that works on `/me`; the same ticket again → 410; the same TOTP code
  on a fresh ticket → 401 (reuse); recovery code → 200, the same recovery code again → 401; wrong password for a 2FA
  user → 401 (unchanged, no ticket); expired ticket (set `expires_at` in the past via SQL) → 410; 5 wrong codes →
  the 6th attempt with the right code → 410; rate limits: tight `mfa_ip`/`mfa_user` → 429, success refunds; the
  ticket isn't stored in plaintext (row holds its SHA-256); two concurrent right-code requests on one ticket → exactly
  one 200.

## Task 4: Startup key check and owner reset (server)

**Files:** `server/src/auth/totp.rs` or `two_factor.rs`, `server/src/lib.rs`, `server/src/main.rs`, `justfile`,
`server/tests/two_factor_startup.rs` (new), `server/tests/common/mod.rs` (if the binary harness needs an env hook).

- [ ] `check_key(db, Option<&TotpKey>) -> anyhow::Result<()>` per spec §3 "Startup and the key"; `serve` calls it
  before binding. Messages contain counts, never secrets or the key.
- [ ] `pulse-app-server reset-2fa <username>` (normalise the name like login does) per spec §3 "Owner reset".
- [ ] `just reset-2fa username` recipe beside `invite-prod`:
  `ssh {{prod}} 'cd /srv/pulse && docker compose exec -T pulse-server pulse-app-server reset-2fa {{username}}'`
  (quote/validate the argument so a stray space or `;` can't run anything else on the host).
- [ ] Tests: `check_key` unit/integration: no 2FA users + no key → ok; enabled user + no key → err; enabled user +
  wrong key → err; right key → ok; only a pending setup + no key → ok. Binary tests (`spawn_binary` style): the real
  binary refuses to start (non-zero exit, no `/health`) with an enabled user and no key; a malformed
  `PULSE_TOTP_KEY` refuses; `reset-2fa alex` turns 2FA off (alex then logs in with the password alone) and
  `reset-2fa nobody` exits non-zero.

## Task 5: Deploy files and README

**Files:** `deploy/env.example`, `deploy/setup.sh`, `deploy/compose.yml`, `deploy/deploy.sh`,
`deploy/deploy-selftest.sh`, `justfile` (`deploy-check`), `.github/workflows/ci.yml` (the same sed),
`deploy/README.md`.

- [ ] `env.example`: `PULSE_TOTP_KEY=` with a comment (encrypts 2FA secrets; setup.sh generates it; keep a copy
  offline with the backup key; changing it locks out 2FA users).
- [ ] `setup.sh`: after the `.env` block, if `PULSE_TOTP_KEY` is absent or empty, set it to `openssl rand -hex 32`
  (append when absent, like `deploy.sh`'s `set_env`); never replace a non-empty value. Don't echo the key.
- [ ] `compose.yml`: `PULSE_TOTP_KEY: ${PULSE_TOTP_KEY:?}` for `pulse-server`.
- [ ] `deploy.sh`: `check_totp_key FILE` (64 hex chars, else a message pointing at the README steps, not echoing the
  value) run by `--check-env` and the real deploy, beside `check_node_ip`. `deploy-selftest.sh` covers it (missing,
  empty, short, non-hex, good).
- [ ] `justfile` `deploy-check` and `ci.yml` sed: fill a dummy key (`PULSE_TOTP_KEY=` + 64 `0`s or similar).
- [ ] `README.md`: `.env` contents mention the key; "First deploy after the two-factor push" steps (re-run
  `just provision ubuntu@pulse` or add the line by hand with `openssl rand -hex 32` as the pulse user, keep a copy
  offline, then `just deploy`); "Two-factor reset" (`just reset-2fa <username>`); the restore/migration section
  notes the key must move with the database.
- [ ] `just deploy-check` passes (needs Docker; it reads only local files).

## Task 6: Client core (Rust)

**Files:** `client/src-tauri/Cargo.toml`, `Cargo.lock`, `client/src-tauri/src/api.rs`,
`client/src-tauri/src/commands.rs`, `client/src-tauri/src/lib.rs`, possibly a new `client/src-tauri/src/qr.rs`.

- [ ] `qrcode = { version = "0.14", default-features = false }`; `qr_matrix(uri) -> QrMatrix { size: u32, modules:
  Vec<bool> }` (row-major, dark = true), error-correction level M.
- [ ] `Api::login` → `LoginOutcome::{Session(SessionResponse), MfaRequired { ticket }}` (a 403 whose body parses as
  `MfaRequired` with `code == "mfa_required"`; any other 403 stays `Forbidden`). `Api::login_mfa(ticket, code)` →
  `MfaOutcome::{Session(SessionResponse), Expired}` (410 → `Expired`; 401 → `ApiError::Rejected("That code didn't
  work. Check the time on your phone and try again.")`). `two_factor_status/setup/enable/disable/regenerate`.
- [ ] Commands: `login` returns `LoginResult` serialised `{status:"ready", user}` | `{status:"mfa_required"}` and
  stores `(Api, ticket)` in `Core` (a `Mutex<Option<…>>`; replaced by any new `login`, cleared by success, `Expired`,
  `register` and `logout`). `login_mfa(code)` → `{status:"ready", user}` | `{status:"expired"}`; no pending ticket →
  `expired`. `two_factor_status`, `two_factor_setup(password)` → `{ secret (grouped in fours with spaces), qr }`
  (the URI never reaches the webview), `two_factor_enable(code)`, `two_factor_disable(code)`,
  `two_factor_regenerate(code)`; signed-in ones go through `core.check` like `logout_others`. Register them in
  `lib.rs`. The ticket and codes are never logged (check `log_failure` and tracing calls).
- [ ] Tests against `pulse_server::testing` (like the existing `api.rs` tests): login for a 2FA user gives
  `MfaRequired`, `login_mfa` with a right code gives a session, wrong → the message above, used ticket → `Expired`;
  a non-2FA login still gives `Session`; setup/enable/disable/regenerate round trip; `qr_matrix` returns a square
  matrix of the expected version size for the URI length and is deterministic; `Core` pending ticket: `login_mfa`
  without one → expired (unit-test the state helper, not Tauri).
- [ ] Windows: `XWIN_ACCEPT_LICENSE=1 cargo xwin clippy -p pulse-client --all-targets --target
  x86_64-pc-windows-msvc -- -D warnings` (in `~/.cargo/bin`).

## Task 7: UI (Svelte)

**Files:** `client/ui/src/lib/tauri.ts`, `client/ui/src/routes/Login.svelte`, `client/ui/src/routes/Login.test.ts`
(new or extended), `client/ui/src/components/TwoFactorSettings.svelte` (new) + `.test.ts`,
`client/ui/src/components/AdvancedSettings.svelte` (+ its test if it needs the new mock).

- [ ] `tauri.ts`: `login` returns `LoginResult`; `loginMfa(code)`; `twoFactor.{status, setup, enable, disable,
  regenerate}` typed with the generated protocol types where they fit and hand-written ones for client-only shapes.
- [ ] Login: after `mfa_required`, the form shows one field (label "Code", placeholder "6-digit code or recovery
  code", `autocomplete="one-time-code"`, `inputmode` text), "Verify" and "Back"; `expired` → back to the password step
  with "That sign-in expired. Log in again."; errors shown as today. The password is cleared from state once the code
  step starts.
- [ ] `TwoFactorSettings.svelte` in Advanced, under its own `<h3>Two-factor authentication</h3>`, flow per spec §4:
  off → "Set up" → password → QR (SVG `<rect>`s from the matrix, white quiet zone of 4 modules, crisp-edges) + the key
  + code → recovery codes once (Copy via `navigator.clipboard.writeText`, guarded; "I've saved these" hides them) and
  "Signed out N other devices." (or "No other devices were signed in."); on → "N recovery codes left", "New recovery
  codes" (code → shows the new set once), "Turn off" (code or recovery code); `available: false` → the not-set-up
  sentence and no button. Cancel at any step returns to the status view and forgets typed values.
- [ ] Tests (vitest, the `invoke` mock pattern of `AdvancedSettings.test.ts`): each flow's commands and arguments,
  the QR renders as many dark `<rect>`s as true modules, recovery codes disappear after "I've saved these" and never
  render again on remount, wrong-code errors shown, `available: false` text, the login code step (mfa_required →
  code field; ready → `onAuthed`; expired → password step with the message).
- [ ] `pnpm check`, `pnpm test`, `pnpm audit --prod --audit-level high` (no new npm dependencies).

## Task 8: SECURITY.md

**Files:** `SECURITY.md`.

- [ ] §4 "Accounts": a "Two-factor authentication" entry describing exactly what was built: optional TOTP (SHA-1, 6
  digits, 30 s, ±1 step, a code can't be reused), the secret encrypted with ChaCha20-Poly1305 under
  `PULSE_TOTP_KEY` (what it protects: database files and backups; what it doesn't: someone with the server's `.env`),
  the server refusing to start without the right key, recovery codes (10, single use, ~79 bits, SHA-256), the login
  ticket (5 min, single use, 5 wrong codes), enabling signs out other devices, rate limits (add `mfa_ip`/`mfa_user` to
  the rate-limit list), owner reset on the host only, no one else sees whether 2FA is on. §3 if the client keeps
  anything new (it keeps the ticket in memory only: say so if §3 lists in-memory secrets).
- [ ] §7: remove "No multi-factor authentication"; add "Two-factor authentication is optional" and "TOTP codes can
  be phished (a fake login page can relay them within the 5-minute window); no WebAuthn/passkeys."
- [ ] Every statement checked against the code on the branch.
