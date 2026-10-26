# Two-factor authentication (TOTP): design

Date: 2026-10-26. Status: the owner approved the description in §1 in chat. §2 onwards are the builder's decisions,
each marked **(builder)**, made so the feature can be built without another round of questions; the final report
lists them with what each costs if wrong.

## 1. Approved description

- Optional per user, authenticator-app TOTP (RFC 6238, 6 digits, 30 s, SHA-1 for app compatibility, e.g. Authy;
  accept ±1 step). Uses the `totp-rs` crate.
- Settings → Advanced → "Two-factor authentication": confirm password → show a QR code (otpauth URI; the QR is
  rendered client-side with a small vetted library, no network) and the manual key → confirm one code to enable. On
  enable: 10 single-use recovery codes, shown once (stored hashed), can be regenerated. Enabling signs out the
  user's OTHER devices (reusing "sign out all other devices").
- Login: password correct + 2FA on → no session yet. The server answers `mfa_required` with a short-lived (5 minute)
  single-use ticket; the client asks for a 6-digit code or a recovery code; ticket + code → session. Code attempts
  are rate-limited (`limits.rs` style, per user and per IP); a TOTP code can't be reused (the last used step is
  remembered); recovery codes are single-use. Sessions that exist when 2FA is turned on stay signed in, except the
  user's other devices, which enabling signs out (above).
- Turning 2FA off needs a current TOTP code or a recovery code. Regenerating recovery codes needs a current TOTP code.
- Owner reset for a locked-out user: a server CLI subcommand run on the box (`just reset-2fa <username>` execs into
  the container). No admin UI. Documented in `deploy/README.md`.
- TOTP secrets are stored encrypted at rest with a new server key from the environment (`PULSE_TOTP_KEY`, 32 bytes),
  AEAD with a random nonce per secret. The server refuses to start if any user has 2FA and the key is missing or
  wrong. The key goes into `deploy/env.example`, `setup.sh` (generated on provision if absent), and README "first
  deploy after this push: add the key" steps. Secrets, codes and tickets are never logged.
- No one else can see whether a user has 2FA.
- No Have-I-Been-Pwned or any other outbound call.
- `SECURITY.md`: MFA moves from the known gaps to the protections, accurately.
- Migration `0007` (after `0006_livekit_grants.sql`).

## 2. Parameters (builder)

| Thing | Value | Why |
|---|---|---|
| Secret | 20 random bytes (160 bits, RFC 4226's recommendation), shown as RFC 4648 base32 without padding | what authenticator apps expect |
| otpauth URI | `otpauth://totp/Pulse:<username>?secret=<B32>&issuer=Pulse&algorithm=SHA1&digits=6&period=30` | built by hand: usernames are `[a-z0-9_.]`, so nothing needs escaping, and it avoids `totp-rs`'s `otpauth` feature (pulls `url`) |
| Code check | steps t-1, t, t+1 (30 s each); constant-time compare; the matched step must be greater than the stored `last_step` | ±1 step, no reuse. A code from step t-1 is refused once t has been used |
| Key | `PULSE_TOTP_KEY`: exactly 64 hex characters (32 bytes), what `openssl rand -hex 32` prints. Set but malformed → the server refuses to start; empty or unset → "missing" | one format, matching how `setup.sh` makes `LIVEKIT_SECRET` |
| AEAD | ChaCha20-Poly1305 (`chacha20poly1305` crate, RustCrypto), 12-byte random nonce per encryption, stored as one BLOB `nonce ‖ ciphertext+tag`; associated data `pulse-totp-v1:<user id>` | a pure-Rust AEAD with no hardware assumptions; the AAD stops a secret being copied to another user's row |
| Recovery codes | 10 codes, each 16 characters from `abcdefghjkmnpqrstuvwxyz23456789` (31 symbols, no `i l o 0 1`; ~79 bits), shown as `xxxx-xxxx-xxxx-xxxx`. Input is normalised (spaces and `-` removed, lowercased) | easy to read off paper, too much entropy to guess |
| Recovery code storage | SHA-256 (hex) of the normalised code, one row per code; a used code's row is deleted | **justification:** the codes are ~79 bits of uniform randomness, so a slow hash adds nothing: even with the database in hand, 2^78 SHA-256 guesses per code are out of reach. argon2 would cost ~19 MiB and tens of ms per check and turn one recovery attempt into ten hashes for no gain. Online guessing is bounded by the rate limits |
| Login ticket | 256-bit random, base64url; only its SHA-256 is stored; expires 5 minutes after the password step; deleted on success; also deleted after 5 wrong codes | single-use, short-lived. A wrong code doesn't burn the ticket at once, so a typo doesn't force retyping the password |
| Rate limits | `mfa_ip`: 5 per minute per client IP; `mfa_user`: 10 per hour per user id. Spent up front, refunded on success, a refused attempt costs nothing (the login pattern) | the same shape as `login_ip` / `login_user` |
| Enable/disable limits | the same two buckets cover every 2FA check made by a signed-in user (password at setup, code at enable, off, regenerate) | a stolen session can't brute-force the password or codes through these endpoints |

## 3. Server

### Storage (migration `0007_two_factor.sql`) (builder)

- `user_totp(user_id PK → users ON DELETE CASCADE, secret_enc BLOB NOT NULL, enabled_at TEXT NULL, last_step INTEGER
  NOT NULL DEFAULT -1, created_at TEXT NOT NULL)`. A row with `enabled_at IS NULL` is a pending setup (a new setup
  overwrites it); 2FA is on when `enabled_at` is set.
- `recovery_codes(user_id → users ON DELETE CASCADE, code_hash TEXT NOT NULL, PRIMARY KEY (user_id, code_hash))`.
- `mfa_tickets(ticket_hash TEXT PK, user_id → users ON DELETE CASCADE, expires_at TEXT NOT NULL, failures INTEGER NOT
  NULL DEFAULT 0)`. Expired rows are pruned on each password step that issues a ticket.
- Tickets live in the database, not memory, for the same reason sessions do: one source of truth, atomic single use.

### HTTP (builder)

All bodies are JSON. Errors for a **signed-in** caller never use 401: the client treats a 401 outside login as "the
session died" and signs out. Wrong password or code there is `403` with a message.

| Route | Auth | Body → answer |
|---|---|---|
| `GET /auth/2fa` | session | → `{ enabled, available, recovery_codes_left }` (only the caller's own state; `available` = the server has a key) |
| `POST /auth/2fa/setup` | session | `{ password }` → `{ secret, otpauth_uri }`. 409 if already on; 403 "wrong password"; 403 if the server has no key |
| `POST /auth/2fa/enable` | session | `{ code }` (TOTP only) → `{ recovery_codes: [10], signed_out }`. 409 if no pending setup or already on; 403 "wrong code". Signs out the caller's other sessions exactly as `logout-others` does (sockets closed with 4001, LiveKit grants removed) |
| `POST /auth/2fa/disable` | session | `{ code }` (TOTP or recovery) → 204. 409 if off; 403 "wrong code". Deletes the secret, the recovery codes and the user's tickets |
| `POST /auth/2fa/recovery-codes` | session | `{ code }` (TOTP only) → `{ recovery_codes: [10] }` replacing all old ones. 409 if off; 403 "wrong code" |
| `POST /auth/login` | none | unchanged for users without 2FA. Password right + 2FA on → **403** `{ code: "mfa_required", message, ticket }` |
| `POST /auth/login/2fa` | none | `{ ticket, code }` (TOTP or recovery) → `SessionResponse`. 410 if the ticket is unknown, expired, used or worn out; 401 wrong code; 429 rate limited |

- **Why 403 for `mfa_required` (builder):** the body is a superset of the normal error body, so a client from
  before this change shows its message ("enter the code from your authenticator app; update Pulse if it doesn't ask
  for one") instead of failing to parse a 200. New clients read `code` and `ticket`.
- A code is a TOTP code when, with spaces removed, it is exactly 6 ASCII digits; anything else is tried as a
  recovery code (where allowed).
- The `mfa_required` answer comes only after the password checks out, so it tells nobody who doesn't know the
  password whether 2FA is on. Nothing else (`/me`, member lists, the gateway) carries 2FA state.
- The password step's login rate-limit tokens are refunded on `mfa_required` (it isn't a failed password), as today
  for a successful login.
- Recovery code use at login and at disable is an atomic `DELETE … WHERE user_id AND code_hash` that must affect one
  row; TOTP use is an atomic `UPDATE user_totp SET last_step = ? WHERE user_id = ? AND last_step < ?`; ticket use is
  an atomic delete. Two racing requests with the same code or ticket: one wins.

### Startup and the key (builder)

- `Config.totp_key: Option<TotpKey>` from `PULSE_TOTP_KEY`.
- `serve` runs a check before it listens: if any user has 2FA on (`enabled_at` set) and there's no key →
  refuse ("N accounts use two-factor authentication but PULSE_TOTP_KEY is not set"). If there is a key, every enabled
  secret must decrypt with it → otherwise refuse ("PULSE_TOTP_KEY does not match …"). Pending setups are not
  checked (a bad one just fails at enable and the user starts again).
- Without a key and with no 2FA users the server runs, and `/auth/2fa` says `available: false`.

### Owner reset (builder)

`pulse-app-server reset-2fa <username>`: deletes that user's `user_totp` row, recovery codes and tickets; prints
`two-factor turned off for <username>: they can sign in with their password and set it up again`. Unknown user → error,
non-zero exit. It doesn't need the key (an unset one is fine; one set but not 64 hex characters stops it, as it stops
every command) and doesn't touch sessions. `just reset-2fa <username>` runs it in the production container like
`just invite-prod`.

## 4. Client (builder)

- **Rust core.** `Api::login` returns either a session or "MFA required" (ticket). The `login` command keeps the
  ticket (and the server it belongs to) in `Core`, in memory only, and tells the UI `mfa_required`; the webview never
  sees the ticket. `login_mfa(code)` sends ticket + code; success activates the session like a normal login; 410
  clears the pending ticket and the UI goes back to the password step; 401 → "That code didn't work…".
- **QR.** The `qrcode` crate (MIT/Apache, `default-features = false`, no dependencies) in `pulse-client` turns the
  otpauth URI into a module matrix; the Settings component draws it as an inline SVG of `<rect>`s. No network, no
  `{@html}`. The webview gets the matrix plus the manual key (grouped in fours for reading), not the URI.
- **Login screen.** After `mfa_required`: one field, "6-digit code or recovery code", a Verify button, a Back link.
- **Settings → Advanced → Two-factor authentication.** Off: a sentence and "Set up". Setup: password → QR + key +
  code → recovery codes (shown once, Copy button, "I've saved these") and "Signed out N other devices". On:
  "N recovery codes left", "New recovery codes" (asks for a code), "Turn off" (asks for a code or recovery code).
  `available: false` → "Two-factor authentication isn't set up on this server yet."

## 5. Deploy and docs (builder)

- `deploy/env.example`: `PULSE_TOTP_KEY=` with a comment. `setup.sh`: if the line is missing or empty, fill it with
  `openssl rand -hex 32` (idempotent, never replaces an existing value). `compose.yml`: `PULSE_TOTP_KEY:
  ${PULSE_TOTP_KEY:?}`. `deploy.sh --check-env` (and the real deploy) refuse when it isn't 64 hex characters, so the
  first deploy after this push can't silently go out without it. `just deploy-check` and CI fill a dummy key.
- `deploy/README.md`: the key in the `.env` description, "first deploy after this push" steps, the reset command,
  and "keep a copy of the key with your backups' age key: a restored database without it locks every 2FA user out
  (fix: reset-2fa each)". The nightly backups contain the encrypted secrets, not the key.
- `SECURITY.md`: §4 gains "Two-factor authentication" (what is stored and how, limits, single use, reset, what the
  key protects against and what it doesn't: it lives in `.env` on the same host); §7 drops "No multi-factor
  authentication" and gains "2FA is optional" and "TOTP is phishable".

## 6. Out of scope

WebAuthn/passkeys, remembering devices, forcing 2FA on, an admin UI, email or SMS codes.
