# Reactions, emoji and the media store: implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** reactions on messages, a Discord-style emoji picker (reactions and composer), per-server custom emoji
(animated too), and the media store they sit on.

**Architecture:**
- **Server:** a `media` module stores content-addressed files on disk, keyed by SHA-256 of the processed bytes. It
  processes uploads with `image` + libwebp and serves `GET /media/{hash}` behind the normal bearer login.
- **Custom emoji:** rows that point at media. Reactions go through one toggle function with a per-channel reaction mode.
- **Client:** the Rust core registers a `pulse-media` URI scheme. It fetches media with the session's bearer token,
  keeps an on-disk cache, and hands the bytes to the webview, so `<img src="pulse-media://…">` works, needs a login and
  downloads once per device.
- **UI:** standard emoji are bundled Twemoji SVGs with emojibase metadata.

**Tech Stack:**
- **Server:** Rust axum 0.8 + sqlx/SQLite, `image` 0.25, `webp-animation` 0.10 (libwebp), `emojis` 0.9, `rustix` 1
  (statvfs).
- **Client:** Tauri 2 custom URI scheme.
- **UI:** Svelte 5, `@twemoji/svg` 15.0.0, `emojibase-data`.

**Spec:** `docs/superpowers/specs/2026-10-22-reactions-emoji-design.md`. Executors read the spec and this plan.

## Global Constraints

- **Reactions:**
  - At most **20** distinct emoji per message.
  - The reaction tooltip names up to **10** people, then "and N others".
- **Custom emoji:**
  - Names: `^[A-Za-z0-9_]{2,32}$`, unique per server (case-insensitive).
  - At most **100** live emoji per server.
- **Uploads:**
  - **2 MB** before processing.
  - Allowed real types (sniffed by magic bytes, never by extension): PNG, JPEG, GIF, WebP.
  - Output fits **128×128**, keeps its aspect ratio, and is padded transparent to square.
  - Still images become WebP. Animations become animated WebP, keeping frame timing, plus a still first-frame WebP.
  - Animations are refused above **300** frames, or above **512 KB** of output.
  - Metadata is stripped. The original is never stored.
  - Decode limits: at most 4096×4096, and an allocation cap of 256 MB, to stop decompression bombs.
  - Rate limit: **30 uploads per hour** per user (`limits.rs` style).
- **Disk:**
  - Uploads are refused with a clear message when the media directory's filesystem has **< 2 GB free**.
  - Log a warning below 5 GB.
- **Media fetch:**
  - Needs a valid login.
  - Responses carry `Cache-Control: private, max-age=31536000, immutable`.
  - Content-addressed: the bytes behind a hash never change.
- **Media location:**
  - The `location` column exists, and `disk` is its only value in this push.
  - The media directory is `PULSE_MEDIA_DIR`, default `/data/media`, i.e. `/srv/pulse/data/media` on the host.
- **Frequently used:**
  - The score decays with a **14-day half-life**: `score = score·0.5^(Δdays/14) + 1` on each use.
  - Ready carries the top **27** (3 rows of 9).
- **Messages:**
  - Custom emoji in message text are the token `<:name:id>`; animated ones are `<a:name:id>`.
  - Standard emoji stay plain Unicode.
  - A message of only emoji (≤ **27**, whitespace allowed) renders large.
- **Public repo:** no IPs, secrets, emails or friends' names in code, comments, docs or test data.

## Review Focus

1. **Emoji tokens inside code spans/blocks, or escaped:** they must render as text, not images. This is pinned in
   Task 9.
2. **A reaction on a deleted message, or on one in a channel you can't see:**
   - Deleted → 404.
   - Unseen → 404, indistinguishable from missing.
   - Reaction events must never reach non-members of a DM.

   This is pinned in Task 3.
3. **Two quick clicks on the same reaction, or the same user on two devices:** the toggle must be idempotent and the
   count must never go negative or drift between clients. Events carry the new absolute count, not a delta. Pinned in
   Task 3 and Task 6.
4. **Malicious or odd uploads:**
   - a GIF declaring 60000×60000;
   - a PNG renamed `.gif`;
   - a 1-frame "animated" GIF, which must be stored as still;
   - a zero-byte body.

   Each needs a clear 400, with no panic and no huge allocation. Pinned in Task 1.
5. **A deleted custom emoji, or a token naming an unknown id**, on an old message or reaction: it shows a muted
   `:name:` chip, and never a broken image. Pinned in Tasks 6 and 9.

---

## Decisions this plan makes (§8 "left to the builder")

- **D1. Media authorisation goes through the Tauri core.**
  - A `pulse-media` URI scheme: on Windows the webview sees `http://pulse-media.localhost/<hash>`, elsewhere
    `pulse-media://localhost/<hash>`.
  - The core fetches with the bearer token, caches on disk and serves.
  - The CSP `img-src` gains exactly those two origins.
  - No signed URLs and no cookies, so the server's auth model doesn't change.
- **D2. Images are processed with `image` 0.25 and encoded with `webp-animation` 0.10.**
  - `image` decodes, sniffs, resizes and supplies the GIF/WebP frame iterators.
  - `webp-animation` encodes, lossy at quality 80, for still and animated alike; one encoder for both.
  - libwebp is compiled from source by its `-sys` crate. If its build needs `clang`, add it to the Dockerfile's build
    stage only.
- **D3. The standard set is Twemoji (Discord's set), using `@twemoji/svg` 15.0.0 with `emojibase-data` for names,
  keywords, groups and skin tones.**
  - A generator script drops any emojibase entry without a Twemoji SVG.
  - The SVGs are copied into `client/ui/public/emoji/` at build time (gitignored).
  - Twemoji's CC-BY 4.0 graphics need attribution; it goes in the About text and in `THIRD_PARTY.md`.
- **D4. Uploading is two steps:**
  - `POST /media/emoji` processes and stores the file, then returns `MediaInfo`. The client previews it through
    `pulse-media`.
  - `POST /servers/{id}/emoji {name, media_hash}` creates the emoji.
  - The hourly cleanup deletes media that nothing references, but only once it is more than 24 h old, so a pending
    preview survives.
- **D5. Custom emoji are soft-deleted** (`deleted_at`), so old reactions and messages can still show `:name:`.
- **D6. Ready carries every server's custom emoji**, deleted ones included (without a hash), plus your frequently
  used.
  - All servers are visible to every group member (`audience.rs`), so this matches the privacy model.
  - The picker only offers emoji from servers you're a member of.
- **D7. Server settings is a new modal, opened from the server-name header in the channel panel.** It has one page,
  Emoji. Discord opens it the same way.

## File map

**Server**
- `server/migrations/0005_media_emoji.sql`: tables `media`, `custom_emoji`, `emoji_usage`; column
  `channels.reaction_mode`.
- `server/src/media/mod.rs`: store (hash, put, path, disk guard, cleanup).
- `server/src/media/process.rs`: sniff, decode, fit, encode.
- `server/src/media/routes.rs`: `POST /media/emoji`, `GET /media/{hash}`.
- `server/src/emoji/mod.rs`, `server/src/emoji/routes.rs`: custom emoji CRUD, `custom_for_ready`, usage scoring.
- `server/src/messages/reactions.rs`: toggle, summaries, reactor list routes.
- Modify:
  - `server/src/lib.rs`: routers, and the cleanup task in `serve`;
  - `server/src/config.rs`: `media_dir`;
  - `server/src/limits.rs`: `upload_user`;
  - `server/src/messages/routes.rs`: reactions in `list`/`load`, usage on `send`;
  - `server/src/gateway/socket.rs`: Ready fields;
  - `server/Cargo.toml`;
  - `deploy/server.Dockerfile` (only if clang is needed);
  - `deploy/backup.sh`;
  - `deploy/README.md`.
- Tests:
  - `server/tests/media.rs`, `server/tests/emoji.rs`, `server/tests/reactions.rs`;
  - fixtures in `server/tests/fixtures/` (tiny PNG/JPEG/GIF/animated GIF/WebP, generated by a test helper rather than
    committed binaries where possible).

**Protocol**
- `protocol/src/rest.rs`: `ReactionSummary`, `Message.reactions`, `MediaInfo`, `CustomEmoji`, `CreateEmojiRequest`,
  `RenameEmojiRequest`, `ChannelKind` untouched.
- `protocol/src/gateway.rs`: `Ready.custom_emoji`, `Ready.frequent_emoji`; events `ReactionAdded`, `ReactionRemoved`,
  `EmojiCreated`, `EmojiUpdated`, `EmojiDeleted`.

**Client core**
- `client/src-tauri/src/media.rs`: the scheme handler and the disk cache.
- Modify: `lib.rs` (register the scheme), `api.rs`, `commands.rs`, `tauri.conf.json` (CSP).

**UI**
- `client/ui/scripts/gen-emoji.mjs`: builds `src/lib/emoji-data.json` (committed) and copies the SVGs.
- `client/ui/src/lib/emoji.ts`: lookup, search, skin tones, URLs, tokens.
- `client/ui/src/lib/reactions.ts`: pure helpers for the reaction row.
- `client/ui/src/components/EmojiPicker.svelte`, `ReactionRow.svelte`, `ServerSettings.svelte`, `EmojiSettings.svelte`.
- Modify: `lib/state.ts`, `lib/tauri.ts`, `lib/markdown.ts`, `components/MessageItem.svelte`,
  `components/Composer.svelte`, `routes/Shell.svelte`, `package.json`.

---

### Task 1: Media store and image processing (server)

**Files:**
- Create: `server/migrations/0005_media_emoji.sql`, `server/src/media/{mod,process,routes}.rs`, `server/tests/media.rs`.
- Modify: `server/src/lib.rs`, `config.rs`, `limits.rs`, `Cargo.toml`, `protocol/src/rest.rs`.

**Interfaces:**
- Produces, in `protocol::rest`:

  ```rust
  pub struct MediaInfo {
      pub hash: String,
      pub content_type: String,
      pub width: u32,
      pub height: u32,
      pub frames: u32,
      pub animated: bool,
      pub still_hash: Option<String>,
  }
  ```

- Produces, in `media`:
  - `pub enum MediaKind { Emoji }`, which round-trips as `as_str`/`parse` over `"emoji"`.
  - `pub async fn put(db: &SqlitePool, dir: &Path, kind: MediaKind, bytes: &[u8], meta: &Processed, by: UserId) -> AppResult<String>`
    returns the hash, deduping on the hash.
  - `pub fn path_for(dir: &Path, hash: &str) -> PathBuf` returns `dir/ab/abcdef….webp`, sharded by the hash's first two
    characters.
  - `pub fn disk_ok(dir: &Path) -> AppResult<()>`.
  - `pub async fn cleanup(db, dir, grace: Duration) -> AppResult<u64>`.
- Produces, in `media::process`:
  - `pub struct Processed { pub webp: Vec<u8>, pub still: Option<Vec<u8>>, pub width: u32, pub height: u32, pub frames: u32 }`
  - `pub fn emoji(input: &[u8]) -> Result<Processed, ProcessError>`. `ProcessError` has user-facing `Display`
    messages: `TooBig`, `UnsupportedType`, `Corrupt`, `TooManyFrames`, `OutputTooBig`, `TooLarge` (dimensions).
- **Migration 0005, all of it** (later tasks consume it):

  ```sql
  CREATE TABLE media (hash TEXT PRIMARY KEY, kind TEXT NOT NULL, location TEXT NOT NULL DEFAULT 'disk',
    content_type TEXT NOT NULL, size INTEGER NOT NULL, width INTEGER NOT NULL, height INTEGER NOT NULL,
    frames INTEGER NOT NULL, still_hash TEXT, uploaded_by TEXT REFERENCES users(id), created_at TEXT NOT NULL) STRICT;
  CREATE TABLE custom_emoji (id TEXT PRIMARY KEY, server_id TEXT NOT NULL REFERENCES servers(id) ON DELETE CASCADE,
    name TEXT NOT NULL, media_hash TEXT NOT NULL REFERENCES media(hash), animated INTEGER NOT NULL,
    added_by TEXT REFERENCES users(id), created_at TEXT NOT NULL, deleted_at TEXT) STRICT;
  CREATE UNIQUE INDEX custom_emoji_live_name ON custom_emoji(server_id, lower(name)) WHERE deleted_at IS NULL;
  CREATE TABLE emoji_usage (user_id TEXT NOT NULL REFERENCES users(id), emoji TEXT NOT NULL, score REAL NOT NULL,
    last_used_at TEXT NOT NULL, PRIMARY KEY (user_id, emoji)) STRICT;
  ALTER TABLE channels ADD COLUMN reaction_mode TEXT NOT NULL DEFAULT 'normal';
  ```

  A still first-frame is its own `media` row of the same kind. Its parent points at it through `still_hash`.

- [ ] **Step 1: Write failing processing tests** (unit tests in `process.rs`). Build the inputs in-test with `image`'s
  encoders (`RgbaImage` → PNG/JPEG; a `GifEncoder` with 3 frames at 50 ms and 1 frame):
  - `still_png_becomes_square_webp_within_128`: a 300×150 input gives `width==128 && height==128`, `frames==1`,
    `still.is_none()`, and output starting with `RIFF`/`WEBP`.
  - `animated_gif_keeps_frames_and_timing_and_has_a_still`: `frames==3`, `still.is_some()`; decode the output and
    assert the frame delays are 50 ms.
  - `single_frame_gif_is_still`: `frames==1`, `still.is_none()`.
  - `extension_is_ignored_real_type_wins`: a PNG still works. Arbitrary bytes `b"hello"` give `UnsupportedType`.
  - `empty_and_oversize_are_refused`: `&[]` gives `UnsupportedType`, and 2 MB + 1 byte gives `TooBig`.
  - `dimension_bomb_is_refused`: a GIF header declaring 60000×60000 gives `TooLarge`, without allocating. Hand-craft
    the header bytes.
  - `too_many_frames_is_refused`: 301 one-pixel frames give `TooManyFrames`.
- [ ] **Step 2: Run them.** `cargo test -p pulse-server --lib media::process`. Expected: FAIL to compile, since
  `emoji` isn't defined yet.
- [ ] **Step 3: Implement `process::emoji`.**
  - Sniff with `image::guess_format`, and accept only Png/Jpeg/Gif/WebP.
  - Read the dimensions first, through `ImageReader::with_guessed_format` + `into_dimensions` with `Limits`, before
    decoding.
  - Get animation frames from `GifDecoder`/`WebPDecoder` via `into_frames()`.
  - Fit each frame with `imageops::resize` (Lanczos3) onto a transparent 128×128 canvas, centred.
  - Encode with `webp_animation::Encoder`, which also encodes a single frame for stills.
  - The still is frame 0 encoded alone.
  - Check the output size last.
- [ ] **Step 4: Run them.** Expected: PASS.
- [ ] **Step 5: Write failing store/route tests** in `server/tests/media.rs`, using the `spawn` and `register` helpers.
  Use a temp `media_dir` through `spawn_with(test_config())`, and add `media_dir` to `test_config` as a tempdir.
  - `upload_returns_info_and_dedupes`: POST the same PNG twice to `/media/emoji` (raw body, any `Content-Type`). The
    hash is equal both times, `SELECT COUNT(*) FROM media` is 1, and the file exists at `path_for`.
  - `fetch_needs_login`: `GET /media/{hash}` without auth gives 401; with auth, 200 with
    `content-type: image/webp` and `cache-control: private, max-age=31536000, immutable`.
  - `unknown_or_malformed_hash_is_404`: a hash of 64 hex characters that isn't stored gives 404. `../etc/passwd`
    gives 404.
  - `bad_upload_is_a_clear_400`: `b"hello"` gives 400, and the body's `message` mentions PNG, JPEG, GIF or WebP.
  - `upload_rate_limited`: with `upload_user: Rate::new(1, 0.0)`, the second upload gives 429.
  - `disk_guard_refuses`: make `disk_ok` take its threshold from `Config` (`media_min_free_bytes`), set it to
    `u64::MAX`; the upload gives 507 with a "server storage is full" message.
  - `cleanup_removes_unreferenced_after_grace`: upload, then `cleanup(grace=0)`; the row and file are gone. With a
    grace of 1 h, both remain.
- [ ] **Step 6: Run them.** `cargo test -p pulse-server --test media`. Expected: FAIL.
- [ ] **Step 7: Implement the store and routes.**
  - `put` writes `tmp` then renames, and `INSERT OR IGNORE`s the row.
  - The hash is `sha2::Sha256` → hex of the processed WebP bytes.
  - `disk_ok` uses `rustix::fs::statvfs`. On non-Unix it always returns Ok.
  - Validate the hash in `GET` against `^[0-9a-f]{64}$` before touching the filesystem.
  - Wrap the upload route in `DefaultBodyLimit::max(2 MB + slack)`. `ProcessError` → `AppError::BadRequest`. Add an
    `AppError::InsufficientStorage` (507).
  - `cleanup` deletes media unreferenced by any `custom_emoji.media_hash`, or by a referencing parent's `still_hash`,
    once older than the grace period. Rows go first, then files. A missing file isn't an error.
  - Spawn the hourly cleanup (grace 24 h) in `serve`.
  - Add `limits.upload_user = Rate::per_hour(30)`, and include it in `unlimited()`.
- [ ] **Step 8: Run them.** Expected: PASS. Also run `just check`, and `docker build -f deploy/server.Dockerfile .`
  (amd64) to prove libwebp builds in the image. Add `clang` to the build stage only if that fails.
- [ ] **Step 9: Commit** `feat(server): media store with content-addressed WebP processing`.

### Task 2: Custom emoji (server)

**Files:**
- Create: `server/src/emoji/{mod,routes}.rs`, `server/tests/emoji.rs`.
- Modify: `protocol/src/rest.rs`, `protocol/src/gateway.rs`, `server/src/gateway/socket.rs`, `server/src/lib.rs`,
  `server/src/media/mod.rs` (cleanup references).

**Interfaces:**
- Produces, in `rest`:
  - `pub struct CustomEmoji { pub id: String, pub server_id: ServerId, pub name: String, pub hash: Option<String>, pub still_hash: Option<String>, pub animated: bool, pub added_by: Option<UserId>, pub created_at: String }`.
    `hash: None` means deleted.
  - `CreateEmojiRequest { name: String, media_hash: String }`.
  - `RenameEmojiRequest { name: String }`.
- Produces, in `gateway::Ready`: `custom_emoji: Vec<CustomEmoji>`, `frequent_emoji: Vec<String>`. Both use
  `#[serde(default)]`.
- Produces `Event::EmojiCreated { emoji }`, `EmojiUpdated { emoji }` and `EmojiDeleted { emoji }`. The deleted one
  carries `hash: None`. `channel_id()` is None, so the audience is Everyone.
- Produces in `emoji`:
  - `pub async fn for_ready(db) -> AppResult<Vec<CustomEmoji>>`.
  - `pub async fn load(db, id) -> AppResult<Option<CustomEmoji>>`, which includes deleted ones.
  - `pub async fn record_use(db_or_tx, user, emojis: &[String])`.
  - `pub async fn frequent(db, user, n) -> AppResult<Vec<String>>`.
- Routes:
  - `POST /servers/{id}/emoji` (member only) → `CustomEmoji`.
  - `PATCH /emoji/{id}` and `DELETE /emoji/{id}` (members of its server). Deleting a deleted one gives 404.
- Emoji ids are ULIDs. Reuse the id type pattern in `protocol/src/ids.rs` if it's trivial, otherwise plain `String`.

- [ ] **Step 1: Write failing tests** in `server/tests/emoji.rs`:
  - `create_list_in_ready_and_event`: a member creates `pog` → 200. A second user's gateway gets `EmojiCreated`, and
    a fresh Ready has it with `hash == Some(media hash)`.
  - `names_validated_and_unique_per_server`:
    - `a` → 400, `has space` → 400, 33 characters → 400;
    - `POG` after `pog` in the same server → 409;
    - `pog` in another server → 200.
  - `non_member_cannot_add`: 403.
  - `cap_100`: seed 100 rows directly through SQL, then a create → 400 "this server has 100 emoji".
  - `media_must_exist_and_be_emoji_kind`: an unknown hash → 400.
  - `rename_and_delete`: the rename emits `EmojiUpdated`. The delete emits `EmojiDeleted` with `hash: None`, and Ready
    still lists it with `hash: None`. After a delete, the name `pog` is free again.
  - `deleted_emoji_media_is_cleaned`: delete, then `media::cleanup(grace=0)` removes its media and still.
  - `frequent_ranks_with_decay`: call `record_use` directly. Use `x` ×3 at day 0 and `y` ×1 at day 30 (inject `now`
    through a `record_use_at` test seam). Then `frequent()` is `["y","x"]`, because 3·0.5^(30/14) ≈ 0.68 < 1.
- [ ] **Step 2: Run them.** Expected: FAIL.
- [ ] **Step 3: Implement.**
  - Validate the name with a regex-free check: length 2–32, `is_ascii_alphanumeric() || '_'`.
  - Map the unique-index violation to 409.
  - `record_use` is an upsert with the decay formula in Rust: read, compute, write inside the caller's transaction.
  - Wire `for_ready` and `frequent(…, 27)` into `build_ready`.
  - Regenerate TS with `just gen-types`.
- [ ] **Step 4: Run them.** Expected: PASS, plus `just check`.
- [ ] **Step 5: Commit** `feat(server): per-server custom emoji with usage ranking`.

### Task 3: Reactions (server)

**Files:**
- Create: `server/src/messages/reactions.rs`, `server/tests/reactions.rs`.
- Modify: `server/src/messages/{mod,routes}.rs`, `protocol/src/{rest,gateway}.rs`.

**Interfaces:**
- Produces:
  - `rest::ReactionSummary { pub emoji: String, pub count: u32, pub me: bool }`.
  - `Message.reactions: Vec<ReactionSummary>`, with `#[serde(default)]`, in first-reacted order.
  - `Event::ReactionAdded { channel_id, message_id, emoji: String, user_id, count: u32 }`, and the same shape for
    `ReactionRemoved`. `channel_id()` returns it, so DM privacy follows `audience.rs`.
  - `pub enum ReactionMode { Normal, OnePerPerson }`, read from `channels.reaction_mode`.
  - `pub async fn toggle(s: &AppState, me: UserId, msg: MessageId, emoji: &str, add: bool) -> AppResult<()>`. This is
    the one function the quotes push will extend.
  - `pub async fn summaries(db, me, ids: &[MessageId]) -> AppResult<HashMap<MessageId, Vec<ReactionSummary>>>`.
- Routes:
  - `PUT /messages/{id}/reactions/{emoji}` adds; it's idempotent, so already-present gives 204.
  - `DELETE /messages/{id}/reactions/{emoji}` removes; idempotent.
  - `GET /messages/{id}/reactions/{emoji}` returns `Vec<UserId>`, oldest first, at most 100.
  - `{emoji}` is percent-encoded Unicode or `custom:<id>`.
- **The emoji key is valid when:**
  - it's `emojis::get(key).is_some()`, after stripping skin-tone modifiers for the lookup and keeping them in the key;
  - or it's `custom:<id>` with a live custom emoji. A deleted one can be *removed* but not *added*.

- [ ] **Step 1: Write failing tests** in `server/tests/reactions.rs`:
  - `add_remove_and_counts`:
    - two users add 👍, so the list shows `count 2, me true`;
    - one removes it, so the count is 1;
    - adding twice still leaves 1 row.
  - `events_carry_absolute_count`: a gateway client sees `ReactionAdded{count:1}`, then `{count:2}`, then
    `ReactionRemoved{count:1}`.
  - `dm_reactions_only_reach_members`: a third user's socket receives nothing within 300 ms. Follow the pattern in
    `privacy.rs`.
  - `cannot_react_where_you_cannot_see`: reacting on a message in someone else's DM → 404.
  - `deleted_message_is_404`.
  - `cap_20_distinct`: 20 different emoji are fine, the 21st → 400. Adding a 2nd user's existing emoji at the cap →
    204.
  - `unknown_emoji_rejected`: `abc` → 400; `custom:<unknown>` → 400; `custom:<deleted>` add → 400 but remove → 204.
  - `one_per_person_mode_replaces`: `UPDATE channels SET reaction_mode='one_per_person'`, then add 👍 then 😂. The
    user holds only 😂, and the events are `ReactionRemoved 👍` then `ReactionAdded 😂`.
  - `reactors_list`: GET returns user ids in order.
  - `reacting_records_usage`: after reacting, `emoji::frequent` contains the key.
  - `list_and_load_carry_summaries`: `GET /channels/{id}/messages` returns reactions with `me` per caller.
- [ ] **Step 2: Run them.** Expected: FAIL.
- [ ] **Step 3: Implement.**
  - Do it in one `begin_write` transaction: load the message and check access through `channel_for`, the same rule
    as `send`, including server membership for text channels.
  - Check the cap. In one-per-person mode, delete the user's other reactions on that message first.
  - Insert or delete, count, `record_use`, commit, then publish the events.
  - In `routes.rs`, call `summaries` from both `list` and `load`. `load` serves no particular user and is used for
    events, so give it reactions with `me:false`. Clients compute `me` from the events' `user_id`.
- [ ] **Step 4: Also record usage on send.** Count `<:name:id>` / `<a:name:id>` tokens as `custom:<id>`, and Unicode
  emoji found with `emojis` by scanning grapheme clusters (add `unicode-segmentation`). Test: `send_records_usage`.
- [ ] **Step 5: Run them.** Expected: PASS, plus `just check`.
- [ ] **Step 6: Commit** `feat(server): message reactions with live counts and a per-channel reaction mode`.

### Task 4: Backups include media

**Files:** modify `deploy/backup.sh` and `deploy/README.md`.

- [ ] **Step 1:** In `backup.sh run`, after the database snapshot:
  - `tar -C data -czf "$tmp/media-$ts.tgz" media`, skipped if `data/media` doesn't exist yet;
  - `age`-encrypt it to the same recipient;
  - upload it to `daily/` next to the db; the weekly copy and pruning work the same way.
  - The trap removes the plaintext tarball.
  - `latest` keeps printing the db object. Add `latest-media`.
- [ ] **Step 2:** Add a media step to README's "Restore drill": fetch, decrypt, `tar -xzf` into `data/`, restart.
- [ ] **Step 3:** `just deploy-check`. Expected: shellcheck clean.
- [ ] **Step 4: Commit** `feat(deploy): nightly backups include the media directory`.

### Task 5: `pulse-media` scheme and client commands (core)

**Files:**
- Create: `client/src-tauri/src/media.rs`.
- Modify: `client/src-tauri/src/{lib,api,commands}.rs`, `client/src-tauri/tauri.conf.json`, `client/ui/src/lib/tauri.ts`.

**Interfaces:**
- Produces `register_asynchronous_uri_scheme_protocol("pulse-media", …)`.
  - The path `/<hash>` must match `^[0-9a-f]{64}$`, otherwise 404.
  - The cache is `<app_cache_dir>/media/<ab>/<hash>.webp`.
  - On a cache miss: `Api::media(hash) -> Result<Vec<u8>, ApiError>`, then verify `sha256(bytes) == hash` before
    writing it to the cache (`tmp` + rename) and serving.
  - Responses are `image/webp` with `Cache-Control: max-age=31536000, immutable`.
  - When logged out it returns 401, with no network call.
- Produces these API/commands, each a thin wrapper over a REST route from Tasks 1–3:
  - `upload_emoji_media(bytes: Vec<u8>) -> MediaInfo`
  - `create_emoji(server_id, name, media_hash) -> CustomEmoji`
  - `rename_emoji(id, name) -> CustomEmoji`
  - `delete_emoji(id)`
  - `add_reaction(message_id, emoji)`
  - `remove_reaction(message_id, emoji)`
  - `list_reactors(message_id, emoji) -> Vec<UserId>`
- Produces the TS side:
  - `api.*` wrappers of the same names in `tauri.ts`;
  - `mediaUrl(hash: string): string` in `lib/emoji.ts` (stubbed here, used in Task 6). It returns
    `http://pulse-media.localhost/<hash>` when `navigator.userAgent` includes `Windows`, else
    `pulse-media://localhost/<hash>`.
- The CSP `img-src` becomes `'self' data: pulse-media: http://pulse-media.localhost`. Also update
  `tauri.dev.conf.json` if it exists.

- [ ] **Step 1: Failing unit tests** in `media.rs` for the pure parts:
  - `parses_only_hex_hashes`: the path → `Option<&str>`.
  - `cache_path_is_sharded`.
  - `rejects_bytes_that_dont_match_hash`, through a `verify(hash, bytes) -> bool`.
- [ ] **Step 2: Run them.** `cargo test -p pulse-client --lib media`. Expected: FAIL.
- [ ] **Step 3: Implement the handler and commands.** The scheme handler gets the `Core`'s current session through
  `app.state::<Core>()`, the same accessor the commands use. Register the commands in `generate_handler!`.
- [ ] **Step 4: Run them.** Expected: PASS, then `just check` and
  `cargo xwin clippy -p pulse-client --all-targets --target x86_64-pc-windows-msvc -- -D warnings`.
- [ ] **Step 5: Commit** `feat(client): pulse-media scheme with a login-gated disk cache`.

### Task 6: Emoji data and UI state

**Files:**
- Create: `client/ui/scripts/gen-emoji.mjs`, `client/ui/src/lib/emoji.ts` (+ `.test.ts`), `client/ui/src/lib/emoji-data.json`.
- Modify: `client/ui/package.json` (deps, plus a `gen:emoji` script, run by `predev`/`prebuild`, that copies the SVGs),
  `.gitignore` (`client/ui/public/emoji/`), `lib/state.ts` (+ test), `THIRD_PARTY.md` (create it if missing).

**Interfaces:**
- `emoji-data.json`:
  - `{ groups: {key, label}[], emoji: { u: string, n: string, k: string[], g: number, s?: string[] }[] }`, where
    `u` is the Unicode, `n` the primary shortcode, `k` the keywords plus other shortcodes, `g` the group index, and `s`
    the 5 skin-tone variants.
  - It only holds entries with a Twemoji SVG.
  - Its committed size must stay under 600 KB.
- `emoji.ts` exports:
  - `byShortcode(name): Std | undefined`
  - `search(q, custom: CustomEmoji[]): Array<Std | CustomEmoji>`: prefix matches on names/shortcodes first, then
    keyword matches; at most 50. `:name:` strips the colons.
  - `twemojiUrl(u: string): string`: `/emoji/<codepoints joined by '-' without fe0f>.svg`, Twemoji's file rule.
  - `withSkin(e, tone: 0..5)`
  - `customToken(e: CustomEmoji): string`: `<:name:id>` or `<a:name:id>`.
  - `TOKEN_RE`
  - `mediaUrl(hash)`, from Task 5.
  - `reactionKey(e)`: Unicode, or `custom:<id>`.
- `state.ts`:
  - `AppState.customEmoji: Record<string, CustomEmoji>` and `frequentEmoji: string[]`, set from Ready.
  - `applyEvent` handles:
    - `ReactionAdded`/`Removed`: set that summary's `count` to the event's count; set `me` only when
      `user_id === s.me.id`; drop the summary at 0; append new emoji at the end;
    - `Emoji*`: upsert into `customEmoji`.
  - Messages not loaded are ignored, the same as `MessageUpdated`.

- [ ] **Step 1: Failing tests:**
  - `emoji.test.ts`:
    - `byShortcode('thumbsup')` returns 👍;
    - `search('pog', [custom pog])` returns the custom one first;
    - `twemojiUrl('❤️') === '/emoji/2764.svg'` and `twemojiUrl('👍🏽') === '/emoji/1f44d-1f3fd.svg'`;
    - `customToken` animated gives `<a:…>`;
    - `TOKEN_RE` matches `<:pog:01ABC>` but not `<:p o g:1>`.
  - `state.test.ts`:
    - `ReactionAdded` from another user increments with `me` unchanged;
    - from me, it sets `me:true`;
    - `Removed` to 0 drops the entry;
    - an absolute count from a reordered event wins, e.g. receiving count 2 then count 1 leaves 1;
    - `EmojiDeleted` keeps the entry with `hash:null`.
- [ ] **Step 2: Run them.** `pnpm -C client/ui test`. Expected: FAIL.
- [ ] **Step 3: Implement the generator and the libs.**
  - The generator reads `emojibase-data/en/data.json` and `en/shortcodes/emojibase.json`, plus the `@twemoji/svg`
    file list. Pin `emojibase-data` to the newest version whose emoji Twemoji 15.0 covers, and log the dropped count.
  - Add the Twemoji attribution (CC-BY 4.0) to `THIRD_PARTY.md`.
- [ ] **Step 4: Run them.** Expected: PASS, plus `just ui-check`.
- [ ] **Step 5: Commit** `feat(ui): emoji data, lookup and reaction state`.

### Task 7: Emoji picker

**Files:** create `client/ui/src/components/EmojiPicker.svelte` (+ `EmojiPicker.test.ts` with `@testing-library/svelte`,
the same setup as the existing component tests; add it if none exist).

**Interfaces:**
- Props: `{ onPick: (e: Std | CustomEmoji) => void; onClose: () => void; servers: Server[]; memberOf: Set<string> }`.
  It reads `app.state.customEmoji` and `frequentEmoji`.
- The skin tone is kept in `localStorage` under `pulse.skinTone`, wrapped in try/catch, the same as the other UI
  prefs.
- **Layout (spec §3.1):**
  - search at the top;
  - the left rail: Frequently used, then one icon per server with live emoji that you're a member of, then the 9
    standard groups;
  - collapsible sections;
  - a footer showing the hovered emoji large with its `:name:`;
  - a thin, themed scrollbar;
  - about 360×420 px; custom emoji use `mediaUrl(still_hash ?? hash)` in the grid and animate on hover;
  - frequently used shows ≤ 27.

- [ ] **Step 1: Failing tests:**
  - `search_filters_and_enter_picks_first`: type `thumbs`, press Enter, so `onPick` gets 👍.
  - `rail_click_scrolls_to_section`: the section element has `scrollIntoView` called. Mock it.
  - `arrows_move_highlight`: ArrowRight then Enter picks the second emoji.
  - `escape_closes`.
  - `custom_from_non_member_servers_hidden`.
  - `frequently_used_first`.
  - `skin_tone_applies_to_toned_emoji`.
- [ ] **Step 2: Run them.** Expected: FAIL.
- [ ] **Step 3: Implement.** Virtualise only if the full grid of ~3700 `<img loading="lazy">` is measurably slow (> 50 ms
  to open). If so, render sections lazily as they scroll into view.
- [ ] **Step 4: Run them.** Expected: PASS.
- [ ] **Step 5: Commit** `feat(ui): emoji picker`.

### Task 8: Reaction row on messages

**Files:**
- Create: `client/ui/src/components/ReactionRow.svelte`, `client/ui/src/lib/reactions.ts` (+ test).
- Modify: `MessageItem.svelte`, `TextChannel.svelte` (pass-through only).

**Interfaces:**
- `reactions.ts`:
  - `tooltipText(names: string[], total: number): string` gives `"A, B and C"`, and past 10 names
    `"A, B, … J and 3 others"`.
  - `optimistic(list, key, add, meId)`, the local flip applied before the server confirms; the next event's absolute
    count replaces it.
- `ReactionRow` props: `{ message: Message; meId: string; nameOf: (id) => string }`.
  - It calls `api.addReaction` / `removeReaction`. On error it reverts the optimistic flip and shows the existing
    toast.
  - Hover fetches the reactors once per hover, using `api.listReactors` and caching per message+emoji until the next
    event for it.
  - "+" opens `EmojiPicker` anchored to the row.
  - A custom emoji whose `hash` is null, or which isn't in `customEmoji`, renders as a muted `:name:` chip. The name
    comes from `customEmoji[id]?.name`, falling back to `emoji`.
- `MessageItem`'s hover bar gets a smiley button that opens the picker; picking adds the reaction. The row is hidden
  for deleted messages.

- [ ] **Step 1: Failing tests:**
  - `tooltipText`: 3 names, and 13 names.
  - `optimistic`: add then remove returns to the original.
  - Component: clicking your highlighted chip calls `removeReaction`; a failed call restores the chip.
  - `deleted_custom_shows_name_chip`.
  - `hidden_on_deleted_message`.
- [ ] **Step 2: Run them.** Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run them.** Expected: PASS.
- [ ] **Step 5: Commit** `feat(ui): reactions under messages`.

### Task 9: Emoji in the composer and in messages

**Files:** modify `lib/markdown.ts` (+ test), `components/Composer.svelte`, `lib/mentions.ts` (generalise the
autocomplete helpers, or add `lib/emojiComplete.ts` alongside; prefer the latter so mentions stay untouched).

**Interfaces:**
- `emojiQuery(beforeCaret): string | null` matches `/(^|\s):([\w]{2,})$/`.
- `applyEmoji(text, caret, insert)` inserts the Unicode or the custom token plus a space.
- The autocomplete lists up to 8 matches from `search`. Custom emoji show their server's name.
- The composer's emoji button opens `EmojiPicker` and inserts at the caret.
- `renderMessage`:
  - after markdown, but outside `code`/`pre`, replaces:
    - custom tokens with `<img class="emoji" src={mediaUrl} alt=":name:">`, or the muted chip when it's
      unknown or deleted;
    - Unicode emoji with `<img class="emoji" src={twemojiUrl} alt={u}>`;
  - adds the class `big` when the message is only emoji (≤ 27) and whitespace;
  - extends DOMPurify to allow `img` with `src` only matching `^(/emoji/|pulse-media://localhost/|http://pulse-media\.localhost/)`
    and attributes `class`, `alt`, `src`, `draggable`;
  - keeps the existing image-in-markdown rule (remote images stay text).

- [ ] **Step 1: Failing tests** in `markdown.test.ts`:
  - `custom_token_renders_img`.
  - `unicode_renders_twemoji`.
  - `tokens_in_code_stay_text`: `` `<:pog:1>` `` and a fenced block.
  - `unknown_token_is_name_chip`.
  - `only_emoji_is_big`, and `text_plus_emoji_is_not_big`.
  - `remote_img_src_still_stripped`: `<img src=https://x>` inside the content can't survive.
  - `markdown_image_stays_alt_text`.

  In `emojiComplete.test.ts`: `query_needs_two_chars`, `apply_inserts_token_and_space`.
- [ ] **Step 2: Run them.** Expected: FAIL.
- [ ] **Step 3: Implement.** `renderMessage` currently takes `(message, mentionNames)`. Add a third parameter,
  `customEmoji: Record<string, CustomEmoji>`, and update its callers. Key the existing per-message cache on the
  custom-emoji state too, so a later `EmojiDeleted` re-renders.
- [ ] **Step 4: Run them.** Expected: PASS.
- [ ] **Step 5: Commit** `feat(ui): emoji in the composer and in messages`.

### Task 10: Server settings → Emoji

**Files:**
- Create: `client/ui/src/components/ServerSettings.svelte`, `EmojiSettings.svelte` (+ test).
- Modify: `routes/Shell.svelte`, so the `panel-head` server name becomes a button with a menu holding
  "Server settings".

**Interfaces:**
- `ServerSettings` props: `{ server: Server; onClose }`. It's a modal shaped like `Settings.svelte`, with one section,
  Emoji.
- `EmojiSettings` props: `{ server: Server }`.
  - A grid of the live emoji: image, `:name:`, added-by name, date.
  - Inline rename (Enter saves, Esc cancels) and delete (with an "are you sure?" confirm).
  - **"Add emoji":**
    - a file input with `accept="image/png,image/jpeg,image/gif,image/webp"`;
    - read the file to `Uint8Array`, then `api.uploadEmojiMedia`;
    - show a preview at 128 px and 32 px from `mediaUrl(hash)`;
    - the name field defaults to the file name sanitised to `[A-Za-z0-9_]`, trimmed to 32 characters, and padded to
      2 if needed;
    - Save → `api.createEmoji`.
  - Shows the count `N / 100`. Server errors (`TooBig`, name taken, …) appear inline.

- [ ] **Step 1: Failing tests:**
  - `default_name_from_file`: `"Big Pog!.gif"` gives `Big_Pog`.
  - `upload_then_save_calls_create_with_hash`.
  - `delete_asks_first`.
  - `server_error_shown_inline`.
- [ ] **Step 2: Run them.** Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run them.** Expected: PASS, plus the full `just check`.
- [ ] **Step 5: Commit** `feat(ui): server settings with an emoji page`.

### Task 11: End-to-end check and handoff

- [ ] **Step 1:** `just check` and the Windows clippy, plus `just voice-it` (rerun once on the known cold-start flake).
  This confirms nothing in voice regressed.
- [ ] **Step 2:** `just dev-server` + `just dev-client` on Linux. Then:
  - upload a still and an animated emoji;
  - react with Unicode and custom emoji from two accounts;
  - check live counts;
  - delete an emoji and see its chip;
  - restart the client and confirm, in the server log, that no `/media` requests repeat.

  Record what was checked in the final report.
- [ ] **Step 3:** Owner-gated, not done by the executor. Deploy by sha (server image, the `PULSE_MEDIA_DIR` default
  needs no env change), a Windows installer, and the spec §7 manual test: a dozen real emoji from the group's Discord.

---

## Self-review notes

- **Spec coverage:**
  - §3.1 picker → T7.
  - §3.2 row, tooltip, "+", hover smiley, 20 cap, live updates, deleted chip, reaction mode → T3, T8.
  - §3.3 composer, autocomplete, tokens, big emoji, seeing emoji you can't use → T6, T9. Ready carries every
    server's emoji.
  - §4.1 → T2, T10. §4.2 → T1, T10 (preview).
  - §5.1 → T1, T5. §5.2: disk location, backups, disk guard → T1, T4. §5.3 → T1.
  - §6 data → the migration in T1, plus T2 and T3. §7 tests are spread across the tasks, the manual one in T11.
- **Not in this push (spec non-goals):** GIF/sticker tabs, avatars/banners, R2, reaction notifications.
