-- Two-factor authentication (TOTP). `secret_enc` is `nonce || ChaCha20-Poly1305(secret)` under
-- PULSE_TOTP_KEY, bound to the user id. A row with `enabled_at IS NULL` is a pending setup; 2FA is
-- on once it is set. `last_step` is the newest accepted 30 s step, so a code works only once.
CREATE TABLE user_totp (
    user_id    TEXT PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    secret_enc BLOB NOT NULL,
    enabled_at TEXT,
    last_step  INTEGER NOT NULL DEFAULT -1,
    created_at TEXT NOT NULL
) STRICT;

-- SHA-256 hex of the normalised code; a used code is deleted.
CREATE TABLE recovery_codes (
    user_id   TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    code_hash TEXT NOT NULL,
    PRIMARY KEY (user_id, code_hash)
) STRICT;

-- Login step 2: a password-checked login waits here for its code. Only the SHA-256 of the ticket
-- is stored; it is deleted on success or after too many wrong codes.
CREATE TABLE mfa_tickets (
    ticket_hash TEXT PRIMARY KEY,
    user_id     TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    expires_at  TEXT NOT NULL,
    failures    INTEGER NOT NULL DEFAULT 0
) STRICT;
CREATE INDEX mfa_tickets_user ON mfa_tickets(user_id);
