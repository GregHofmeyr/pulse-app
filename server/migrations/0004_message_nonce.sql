-- Idempotent sends (spec 2026-10-13-optimisation-design, A6): the outbox retries with the same
-- client nonce, so keep at most one message per (author, nonce). Old rows and system lines have none.
ALTER TABLE messages ADD COLUMN nonce TEXT;
CREATE UNIQUE INDEX messages_author_nonce ON messages (author_id, nonce) WHERE nonce IS NOT NULL;
