# Attachments (images and files in chat) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** People can paste, drop or pick up to 10 files per message in text channels, voice chats, DMs and groups: images (converted to WebP on the client), videos, audio and any other file up to 50 MB kept forever, and big temporary sends up to 2 GB that expire after 1 or 7 days.

**Architecture:** An `attachments` table on the server records each upload (pending until a send attaches it to a message). The client asks the server for a presigned PUT (or, for big sends, a multipart upload with presigned parts), uploads straight to R2, then sends the message with the attachment ids. Reads attach the attachment list to `Message`; a per-message route signs short-lived GETs. One sweeper deletes unattached uploads, attachments of deleted messages and expired temp files. Messages without a caption store a plain-text fallback (`📎 report.pdf`) as `content`, so 0.2.x clients, notifications, previews and search all read it.

**Tech Stack:** Rust (axum, sqlx/SQLite, rusty-s3 0.11, reqwest), Tauri 2 Rust core (image 0.25, webp-animation), Svelte 5 + vitest.

**Spec:** `docs/superpowers/specs/2026-11-11-attachments-design.md`

## Global Constraints

- Up to **10 files per message**; normal attachments **up to 50 MB each, kept forever**; big temporary sends **above 50 MB, up to 2 GB per file**, expiring in **1 day or 7 days**; **server-wide cap of 20 GB of live temp files**.
- Where: server text channels, voice channels' chats, DMs and groups. **Not** quotes or clips channels.
- Images are converted to **WebP** on the client (high quality, **longest side capped at 4096 px**; animated GIF/WebP stay animated). The server never decodes an attachment.
- Videos (mp4/webm/mov) are not re-encoded. Any file type is allowed; folders are refused ("Zip the folder first") or zipped in the app.
- Executable types get "This file can run programs on your computer" on Download. Pulse never opens files by itself.
- Presigned URLs are scoped to one object and short-lived (uploads and downloads: 1 h, the clips `URL_TTL`).
- Uploads never attached are swept after 24 h; deleting a message deletes its objects; expired temp files are deleted and the card shows **Expired**.
- 0.2.x clients never see a blank message: with no caption, `content` is the fallback text.
- Migrations: next free number after rebasing on origin/main; foreign keys are OFF at migration time (`server/src/db.rs:68`).
- Existing colour/space tokens only; icon sizes 14/16/18; reduced motion respected; look matches message cards, `LinkEmbed.svelte` (#40) and the clip player.
- Public repo: no bucket names or secrets; test people are alex, sam, kim, jo.
- Cargo: `CARGO_BUILD_JOBS=4 flock ~/Projects/pulse-app/target/.cargo-serial nice -n 19 ionice -c3 cargo …` with `CARGO_TARGET_DIR` in this worktree; tests `-- --test-threads=4`. pnpm via node at `~/.config/nvm/versions/node/v22.22.3/bin`.

## Decisions (builder's calls)

- **R2 layout:** `attachments/<key>` for normal files, `temp/<key>` for temporary sends; `<key>` is `media::new_key()` (64 hex, server-assigned, never the file name). Separate prefixes let `just storage` report by kind and allow an R2 lifecycle rule on `temp/` as a backstop.
- **Stored content type:** the server picks it from the client's declared type through an allow-list (`image/webp`, `video/mp4`, `video/webm`, `video/quicktime`, `audio/mpeg`, `audio/ogg`, `audio/wav`, `audio/flac`, `audio/aac`, `audio/mp4`, `audio/webm`); anything else is stored as `application/octet-stream` with kind `file`. The type is a signed header of the PUT, so the stored object carries exactly it. Images must be `image/webp` (the client converted them); at send the server reads the object's first 12 bytes with a ranged GET and checks `RIFF????WEBP` (a magic check, not a decode).
- **Fallback text** (`attachments::fallback_text`): one file → `📎 <name>`; several, all images → `📎 <n> images`; otherwise `📎 <n> files`. Stored as `content` when the caption is empty. `Message.caption_is_fallback` (skip when false) tells new clients to hide it; the server computes it on read as `content == fallback_text(&attachments)`.
- **Editing** a message with attachments may clear the caption (stored as the fallback again); its files can't change.
- **Downloads:** `GET /messages/{id}/attachments/urls` signs a GET for every live attachment of one message (one call per message, cached client-side like clip links). Expired temp attachments are listed with no URL.
- **Purge hook (#184):** `attachments::keys_in_channels(&mut conn, &[ChannelId]) -> Vec<String>` + `attachments::remove_objects(&AppState, Vec<String>)`; channel purge uses them now. `attachments.message_id`/`channel_id` are `ON DELETE SET NULL`, so any hard delete that misses the hook leaves rows that the pending sweep removes (object then row) once older than 25 h.
- **Rate limits:** new `attachment_user` limiter (`Rate::new(60, 1.0)`) on starting an upload; at most **50 open (unattached) uploads per person**; `clip_url_user` reused for attachment URLs.

## Review Focus

- A send whose attachment PUT never arrived (dropped connection) → a clear "The upload didn't arrive. Try again." 409, nothing posted, the upload still swept later. (Task 3 test `send_with_missing_object_is_409_and_posts_nothing`.)
- Someone attaching another person's upload id, or an upload started for a different channel → 404, nothing attached. (Task 3 test `foreign_or_wrong_channel_upload_is_refused`.)
- A file name with path separators, control characters, an RTL override or 300 characters → stored sanitised and shortened, never trusted as a path when saving on the client. (Task 2 test `names_are_sanitised`; client Task 8 saves through a dialog with the sanitised name.)
- A 0.2.x client receiving an attachment-only message → readable `📎 …` content, never blank, also in notifications. (Task 3 test `attachment_only_message_has_fallback_content`.)
- Deleting a message while a URL is cached → URLs route answers 404 afterwards and the objects are gone. (Task 4 test `deleting_a_message_deletes_its_objects`.)

---

## Slice 1 — normal attachments (≤ 50 MB), images as WebP, video/audio/file cards, has:media

### Task 1: Protocol types

**Files:**
- Modify: `protocol/src/ids.rs` (add `id!(AttachmentId);`)
- Modify: `protocol/src/rest.rs` (new types; `Message` fields; `SendMessageRequest.attachments`; `SearchHas::Media`, `SearchHas::File`)
- Modify: every `Message { … }` literal (`grep -rn "Message {$" server/src client/src-tauri/src protocol/src`) to add `attachments: vec![], caption_is_fallback: false`
- Modify: `protocol/src/lib.rs` `export_all()` (new types); regenerate with `just gen-types`; `just types-check` must pass

**Interfaces — Produces:**
```rust
pub enum AttachmentKind { Image, Video, Audio, File }            // serde lowercase
pub struct Attachment {
    pub id: AttachmentId, pub name: String, pub content_type: String, pub kind: AttachmentKind,
    pub size: u64,
    pub width: Option<u32>, pub height: Option<u32>, pub duration_ms: Option<u32>,
    pub expires_at: Option<String>,   // temp sends only (RFC 3339 millis Z)
    pub expired: bool,                // temp send whose bytes are gone
}
// Message: #[serde(default, skip_serializing_if = "Vec::is_empty")] #[ts(optional)] pub attachments: Vec<Attachment>,
//          #[serde(default, skip_serializing_if = "std::ops::Not::not")] #[ts(optional)] pub caption_is_fallback: bool,
// SendMessageRequest: #[serde(default)] pub attachments: Vec<AttachmentId>,
pub struct StartAttachmentRequest { pub name: String, pub size: u64, pub content_type: String,
    #[serde(default)] pub width: Option<u32>, #[serde(default)] pub height: Option<u32>,
    #[serde(default)] pub duration_ms: Option<u32>,
    #[serde(default)] pub temp_days: Option<u8> }                 // slice 2 uses temp_days
pub struct AttachmentUpload { pub id: AttachmentId, pub url: Option<String>, pub content_type: String,
    pub expires_at: String, #[serde(default)] pub multipart: Option<MultipartInfo> }  // slice 2 fills multipart
pub struct AttachmentUrl { pub id: AttachmentId, pub url: Option<String>, pub expires_at: Option<String> }
pub mod attachment_limits { pub const PER_MESSAGE: usize = 10; pub const MAX_BYTES: u64 = 50 * 1024 * 1024;
    pub const TEMP_MAX_BYTES: u64 = 2 * 1024 * 1024 * 1024; pub const TEMP_CAP_BYTES: u64 = 20 * 1024 * 1024 * 1024;
    pub const NAME_MAX_CHARS: usize = 200; }
```
`MultipartInfo` is defined in slice 2 (Task 10); in slice 1 declare `multipart` only once Task 10 adds the type — slice 1's `AttachmentUpload` has `id, url, content_type, expires_at`.

- [ ] Step 1: serde round-trip test in `protocol/src/rest.rs` tests: a `Message` with no attachments serializes without `attachments` or `caption_is_fallback` keys; one with an attachment round-trips; `SendMessageRequest` JSON without `attachments` deserializes to an empty vec (0.2.x sends).
- [ ] Step 2: run `cargo test -p pulse-protocol` → FAIL; implement; → PASS.
- [ ] Step 3: fix every `Message {` literal; `cargo build --workspace --all-targets` clean; regenerate TS bindings; commit `feat(attachments): protocol types (#57)`.

### Task 2: Migration, attachments module, start-upload route

**Files:**
- Create: `server/migrations/00NN_attachments.sql` (NN = next free after rebase)
- Create: `server/src/attachments/mod.rs` (constants, key/type helpers, `fallback_text`, `attach`, `keys_in_channels`, `remove_objects`)
- Create: `server/src/attachments/routes.rs`
- Modify: `server/src/lib.rs` (module, router merge), `server/src/limits.rs` (`attachment_user`), `server/src/media/r2.rs` (`get_range`)
- Test: `server/tests/attachments.rs`

Migration (STRICT; guard pattern from `0009_clips.sql`):
```sql
CREATE TABLE attachments (
    id           TEXT PRIMARY KEY,
    key          TEXT NOT NULL UNIQUE,              -- R2 object key, e.g. attachments/<64 hex>
    user_id      TEXT NOT NULL REFERENCES users(id),
    channel_id   TEXT REFERENCES channels(id) ON DELETE SET NULL,
    message_id   TEXT REFERENCES messages(id) ON DELETE SET NULL,  -- NULL while pending
    position     INTEGER NOT NULL DEFAULT 0,
    name         TEXT NOT NULL,
    content_type TEXT NOT NULL,
    kind         TEXT NOT NULL CHECK (kind IN ('image', 'video', 'audio', 'file')),
    size         INTEGER NOT NULL,
    width        INTEGER, height INTEGER, duration_ms INTEGER,
    expires_at   TEXT,                               -- temp sends only
    upload_id    TEXT,                               -- multipart upload id (temp sends)
    purged_at    TEXT,                               -- when an expired temp file's object was deleted
    created_at   TEXT NOT NULL
) STRICT;
CREATE INDEX attachments_message ON attachments(message_id, position) WHERE message_id IS NOT NULL;
CREATE INDEX attachments_pending ON attachments(created_at) WHERE message_id IS NULL;
CREATE INDEX attachments_user_pending ON attachments(user_id) WHERE message_id IS NULL;
CREATE INDEX attachments_live_temp ON attachments(expires_at) WHERE expires_at IS NOT NULL AND purged_at IS NULL;
```

**Interfaces — Produces (in `server/src/attachments/mod.rs`):**
```rust
pub const URL_TTL: Duration;                 // = crate::clips::URL_TTL (1 h)
pub const PENDING_TTL_HOURS: i64 = 24;
pub const OPEN_PER_USER: i64 = 50;
pub fn object_key(key: &str, temp: bool) -> String;          // "attachments/<key>" | "temp/<key>"
pub fn stored_type(declared: &str) -> (&'static str, AttachmentKind);   // allow-list above
pub fn clean_name(name: &str) -> Option<String>;   // None when empty after cleaning
pub fn fallback_text(atts: &[Attachment]) -> String;
pub async fn attach(db: &SqlitePool, msgs: &mut [Message]) -> AppResult<()>;   // fills attachments + caption_is_fallback for live messages
pub async fn keys_in_channels(c: &mut SqliteConnection, channels: &[ChannelId]) -> AppResult<Vec<String>>;
pub async fn remove_objects(s: &AppState, keys: Vec<String>);   // object first, then row by key; failures logged
// r2.rs
pub async fn get_range(&self, key: &str, len: u64) -> AppResult<Option<Vec<u8>>>;
```
`clean_name`: drop everything up to the last `/` or `\`, strip control chars and bidi overrides (U+202A–U+202E, U+2066–U+2069), trim, trim leading dots, cut to `NAME_MAX_CHARS` keeping the extension.

Route `POST /channels/{id}/attachments` (`StartAttachmentRequest` → `AttachmentUpload`): limiter `attachment_user`; `storage()` 503 when R2 isn't set up ("File storage isn't set up on this server."); `channel_for` + chat-kind check (`kind.has_chat()` or DM/group; quotes/clips → 400 "Files can't be posted here."); server membership; size 0 → 400 "That file is empty."; size > `MAX_BYTES` without `temp_days` → 413 "Files can be up to 50 MB."; `clean_name` None → 400; `writes_ok`; under `begin_write`, refuse when open uploads ≥ `OPEN_PER_USER` (429 "Too many uploads in progress."); insert pending row (`kind`/`content_type` from `stored_type`; image declared but not `image/webp` → 400 "Images must be WebP."); sign `upload_url(object_key, size, content_type, URL_TTL)`.

Also `DELETE /attachment-uploads/{id}` (cancel: mine and pending → delete row then object, 204; else 404).

- [ ] Step 1: tests in `server/tests/attachments.rs` (use `spawn()`, `register`, `create_server`, a DM via the dms route, and the app's fake S3): `starts_an_upload_in_text_voice_dm_and_group`, `refused_in_quotes_and_clips_channels`, `non_member_is_forbidden`, `empty_and_over_50mb_are_refused` (50 MB exactly is OK), `names_are_sanitised` (`"../../a\u{202e}gpj.exe"` → `"agpj.exe"`, 300-char name → ≤ 200 chars ending `.txt`), `non_webp_image_type_is_refused`, `unknown_types_are_stored_as_octet_stream`, `the_put_url_is_signed_for_one_object_and_one_length` (URL path ends with the row's key, `X-Amz-Expires=3600`, a PUT of a different length is 403), `open_uploads_are_capped_at_50`, `cancel_deletes_row_and_object`.
- [ ] Step 2: run → FAIL; implement migration, module, routes; run → PASS.
- [ ] Step 3: unit tests in `mod.rs` for `fallback_text` (`📎 report.pdf`, `📎 3 images`, `📎 2 files`) and `stored_type`; commit `feat(attachments): uploads table and start route (#57)`.

### Task 3: Sending, reading and editing messages with attachments

**Files:** Modify `server/src/messages/routes.rs` (`send`, `edit`, `load`, `fill`), `server/src/attachments/mod.rs`; Test `server/tests/attachments.rs`.

**Interfaces — Produces:** `pub async fn claim(s: &AppState, me: UserId, channel: ChannelId, ids: &[AttachmentId]) -> AppResult<Vec<Attachment>>` (pre-transaction checks: count ≤ 10 else 400 "Up to 10 files per message."; no duplicates; each row mine, pending, this channel, younger than `PENDING_TTL_HOURS` — anything else 404; `head` size equals the row's size else 409 "The upload didn't arrive. Try again." / "The upload didn't match. Try again."; images pass the WebP magic check else 409 "That image didn't upload correctly."), and `pub async fn bind(tx: &mut SqliteConnection, me: UserId, message: MessageId, ids: &[AttachmentId]) -> AppResult<()>` (inside the send transaction: `UPDATE … SET message_id = ?, position = ? WHERE id = ? AND user_id = ? AND message_id IS NULL`, any 0 rows → 404).

`send`: when `req.attachments` is non-empty the content may be empty; `content = caption or fallback_text(&claimed)`; set `msg.attachments`, `msg.caption_is_fallback`. Threads/replies may carry attachments only where plain messages may. `load`/`fill` call `crate::attachments::attach`. `edit`: a message with attachments accepts empty content → fallback.

- [ ] Step 1: tests: `sends_a_message_with_three_images_and_a_pdf` (order kept, kinds, sizes, `MessageCreated` carries them, list returns them), `attachment_only_message_has_fallback_content` (content `📎 report.pdf`, `caption_is_fallback` true; JSON of a captioned message has no `caption_is_fallback`), `eleven_attachments_are_refused`, `send_with_missing_object_is_409_and_posts_nothing`, `size_mismatch_is_409`, `fake_webp_is_refused` (object of the right size but not RIFF/WEBP), `foreign_or_wrong_channel_upload_is_refused`, `an_attachment_can_only_be_sent_once`, `nonce_retry_returns_the_same_message`, `editing_can_clear_the_caption`, `old_client_send_without_attachments_field_still_works`.
- [ ] Step 2: FAIL → implement → PASS; commit `feat(attachments): send and read messages with files (#57)`.

### Task 4: Download URLs, delete, purge hook, sweeper, has:media

**Files:** Modify `server/src/attachments/routes.rs`, `server/src/messages/routes.rs` (`remove`), `server/src/servers/channels.rs` (`purge`), `server/src/search/routes.rs`; Create `server/src/attachments/sweep.rs`; Modify `server/src/lib.rs` (spawn sweeper next to `clips::sweep::spawn`).

Route `GET /messages/{id}/attachments/urls` → `Vec<AttachmentUrl>` in position order: limiter `clip_url_user`; message live and visible (`channel_for`, server membership) else 404/403; each attachment signed with `download_url(object_key, URL_TTL)`; an expired/purged temp one has `url: None`.

`remove`: collect the message's attachment keys in the transaction; after commit (and after dropping `_order`) `remove_objects`. Channel `purge`: `keys_in_channels` before deleting messages, `remove_objects` after commit (same spawn as clips).

Sweeper (`run(s, now) -> AppResult<SweepReport { pending, deleted, expired }>`, every 10 min, `spawn` only when R2 is set up): (1) rows with `message_id IS NULL` and `created_at < now − 25 h` → abort multipart if `upload_id` (slice 2), delete object, delete row (guarded `AND message_id IS NULL`); (2) rows whose message has `deleted_at IS NOT NULL` → delete object, delete row; (3) `expires_at < now AND purged_at IS NULL` → delete object, set `purged_at`. Object first; a failed delete keeps the row for the next run.

Search: `SearchHas::Media` → `EXISTS (SELECT 1 FROM attachments x WHERE x.message_id = m.id AND x.kind IN ('image','video','audio'))`; `SearchHas::File` → `… x.kind = 'file'`.

- [ ] Step 1: tests: `urls_are_signed_per_object_and_short_lived` (one URL per attachment, path ends with its key, `X-Amz-Expires=3600`, GET returns the bytes), `outsider_gets_404_and_ex_member_403`, `deleting_a_message_deletes_its_objects` (objects gone from fake S3, rows gone, URLs route 404), `purging_a_channel_deletes_its_attachments`, `sweep_removes_unattached_uploads_after_24h` (23 h kept, 26 h gone with its object), `sweep_retries_a_failed_delete` (`fake.set_down(true)` keeps the row), `has_media_and_has_file_filter_search`.
- [ ] Step 2: FAIL → implement → PASS; commit `feat(attachments): links, delete, sweep and has:media (#57)`.

### Task 5: Client core — WebP conversion and staging

**Files:**
- Modify: `client/src-tauri/Cargo.toml` (`image` features add `jpeg`, `gif`, `bmp`; add `webp-animation = { version = "0.10", features = ["static"] }` as the server does; `.cargo/config.toml` already carries the libwebp Windows CFLAGS fix)
- Create: `client/src-tauri/src/attachments/mod.rs`, `client/src-tauri/src/attachments/prepare.rs`
- Modify: `client/src-tauri/src/commands.rs`, `client/src-tauri/src/lib.rs` (register commands)

**Interfaces — Produces:**
```rust
pub struct Staged { pub id: String, pub name: String, pub size: u64, pub content_type: String,
    pub kind: AttachmentKind, pub width: Option<u32>, pub height: Option<u32>,
    pub preview: Option<String> }        // data:image/png;base64 thumbnail, longest side 96 px, images only
// Serialize to the UI; mirrored by hand as `StagedFile` in lib/tauri.ts (like ClipJob).
pub fn to_webp(bytes: &[u8]) -> Result<WebpOut, PrepareError>;  // WebpOut { bytes, width, height, frames }
pub fn content_type_for(name: &str) -> (&'static str, AttachmentKind);  // by extension: mp4/webm/mov, mp3/ogg/opus/wav/flac/aac/m4a; images handled by to_webp
// commands
stage_attachment_paths(paths: Vec<String>) -> Vec<Result-like {staged | error}>   // a directory → error "Zip the folder first." (slice 3 offers zip)
stage_attachment_bytes(raw body, header x-name) -> Staged                         // clipboard paste
pick_attachment_files() -> Vec<String>                                            // multi-select open dialog
unstage_attachment(id: String)
```
`to_webp`: decode png/jpeg/gif/webp/bmp (animated GIF/WebP keep every frame and its delay), resize so the longest side ≤ 4096 (Lanczos3), encode with `webp_animation` lossy quality **90** (one frame for stills). Staged files are written to `<app cache>/attachments/<id>` and removed on unstage, after a successful send, and at startup.  An image that fails to decode is staged as a plain file (`application/octet-stream`, kind file) under its own name. Non-image files are staged by path (not copied).

- [ ] Step 1: unit tests in `prepare.rs`: `a_large_png_becomes_webp_capped_at_4096` (5000×2500 → 4096×2048, output starts with `RIFF`/`WEBP`), `a_small_jpeg_keeps_its_size`, `an_animated_gif_stays_animated` (3 frames in → 3 out), `a_broken_image_is_a_plain_file`, `content_types_by_extension` (`clip.MOV` → video/quicktime, `a.exe` → octet-stream/file), `a_directory_is_refused`.
- [ ] Step 2: FAIL → implement → PASS (`cargo test -p pulse-client attachments`); `cargo xwin clippy` builds libwebp for Windows; commit `feat(attachments): stage files and convert images to WebP (#57)`.

### Task 6: Client core — upload and send job, links, save

**Files:** Create `client/src-tauri/src/attachments/send.rs`; Modify `client/src-tauri/src/api.rs`, `commands.rs`, `lib.rs`.

**Interfaces — Consumes:** Task 1 types, Task 2–4 routes, `clips::upload::{client, put_file, CancellationFlag, UploadError}`, `clips::download::save_url`, `commands::ask_save_path`.
**Produces:**
```rust
// api.rs
start_attachment(token, channel: ChannelId, &StartAttachmentRequest) -> Result<AttachmentUpload, ApiError>
cancel_attachment(token, AttachmentId) -> Result<(), ApiError>
attachment_urls(token, MessageId) -> Result<Vec<AttachmentUrl>, ApiError>
send_message_with(token, channel, &SendMessageRequest) -> Result<Message, ApiError>   // existing send if it already takes the request
// commands
send_with_attachments(channel: ChannelId, content: String, reply_to: Option<MessageId>,
    thread_root: Option<MessageId>, nonce: String, staged: Vec<String>, temp_days: Option<u8>) -> ()
cancel_attachment_send(nonce: String)
attachment_urls(message_id: MessageId) -> Vec<AttachmentUrl>
save_attachment(message_id: MessageId, attachment_id: AttachmentId, name: String) -> ()
```
`send_with_attachments` spawns a job keyed by `nonce`: for each staged file in order, start an upload (once; remembered across retries), `put_file` with progress, then send the message (content may be empty) with the nonce. Emits `"attachments://progress"` `{ nonce, sent, total }` (bytes over all files, ≤ 4/s) and on finish the existing `"pulse://outbox"` `{ nonce, status: 'sent' | 'failed', message }`, so the UI's pending entry resolves the same way as text sends. Calling it again with the same nonce after a failure resumes (already-uploaded files are not uploaded again). `save_attachment` asks with the server's sanitised name, downloads with `save_url`, and retries once with a fresh link on 403. Nothing ever opens a file.

- [ ] Step 1: tests with a mock server (follow how `clips/jobs.rs` tests mock the API): `uploads_each_file_then_sends_with_ids_in_order`, `progress_reaches_total`, `a_failed_put_reports_failed_and_retry_skips_done_files`, `cancel_stops_and_cancels_uploads`.
- [ ] Step 2: FAIL → implement → PASS; commit `feat(attachments): upload and send job (#57)`.

### Task 7: UI — composer: pick, paste, drop, chips, send

**Files:**
- Create: `client/ui/src/components/ComposerFiles.svelte` (+ test), `client/ui/src/lib/attachments.ts` (+ test)
- Modify: `client/ui/src/components/Composer.svelte`, `TextChannel.svelte` (drop target), `RepliesPanel.svelte` (if it passes `onSend`), `client/ui/src/lib/tauri.ts` (commands, `StagedFile`), `client/ui/src/lib/store.svelte.ts` (`queueMessage` takes staged files; pending entry gets `files` + `progress`)

**Interfaces — Produces (`lib/attachments.ts`):** `MAX_FILES = 10`, `MAX_BYTES = 50 * 1024 * 1024`, `formatSize(bytes): string` ("1.4 MB", "820 KB"), `isExecutable(name): boolean` (exe, msi, bat, cmd, com, ps1, psm1, vbs, js, jse, wsf, scr, pif, cpl, msc, reg, lnk, hta, jar, sh, bash, run, bin, appimage, deb, rpm, apk, app, dmg, pkg, command — case-insensitive), `shownCaption(m: Message): string` (empty when `caption_is_fallback`).

`onSend` becomes `(text: string, files: StagedFile[]) => void`; Enter sends when there is text or at least one file. Chips show a thumbnail (images) or a file glyph, name, size and a remove ×; an 11th file shows "Up to 10 files per message."; a file over 50 MB shows "Files can be up to 50 MB." until slice 2 adds temp sends. The "+" button (16 px paperclip) calls `pick_attachment_files` then stages. Paste: `clipboardData.files` → `stage_attachment_bytes`. Drop: `getCurrentWebview().onDragDropEvent` in `TextChannel` (ignored when an `[aria-modal="true"]` is open, as in `ClipsChannel.svelte:199`), with a drop highlight. A pending message shows its chips and a progress bar from `attachments://progress`.

- [ ] Step 1: tests (`QuoteComposer.test.ts` style): `attachments.test.ts` for `formatSize`, `isExecutable` (`Setup.EXE`, `run.AppImage` true; `notes.txt` false), `shownCaption`; `ComposerFiles.test.ts`: chips render and remove; `Composer.test.ts`: paste a PNG stages it, Enter with only a file calls `onSend('', [file])`, 11 files show the limit message, folder error text shows; `TextChannel` drop stages paths.
- [ ] Step 2: FAIL → implement → PASS (`pnpm test`, `pnpm check`); commit `feat(attachments): composer files (#57)`.

### Task 8: UI — showing attachments: grid, viewer, players, file card

**Files:**
- Create: `client/ui/src/components/MessageFiles.svelte`, `ImageViewer.svelte`, `FileCard.svelte` (+ tests), `client/ui/src/lib/linkCache.ts` (+ test)
- Modify: `client/ui/src/lib/clipLinks.ts` (built on `linkCache`, same exports and behaviour), `MessageItem.svelte` (render `MessageFiles` after content, hide fallback content), `ClipPlayer.svelte` (accept an optional `load(fresh: boolean) => Promise<string>` so attachments reuse it), `client/ui/src/components/stacking.test.ts` (list the viewer like the quote lightbox if needed)

**Interfaces:** `linkCache<K>(fetch: (k: K) => Promise<{url: string, expires_at: string}>)` → `{ get(k, {fresh?}): Promise<string>, forget(): void }`, with clipLinks' renew margin and request sharing. `attachmentLink(messageId, attachmentId, {fresh?})` fetches `attachment_urls(messageId)` once per message and caches every URL it returns.

Rendering: one image large (max 400×300, aspect kept from width/height, no layout shift), 2–10 in a grid (2 columns; 3 for ≥ 5); click opens `ImageViewer` (dialog with `use:modal`, `.modal-close`; ←/→ next/previous, wheel/+/− zoom, "Save image" → `save_attachment`, "Open original size" toggles 1:1). Video: `ClipPlayer` with `load`. Audio: compact `<audio controls>` in a card. Other files: `FileCard` styled like `LinkEmbed` (icon 18, name, size, Download); executable names show "This file can run programs on your computer" next to Download. A broken image falls back to a file card. Reduced motion: the viewer opens without its scale animation.

- [ ] Step 1: tests: `linkCache.test.ts` (shares one request, renews before expiry, `fresh` replaces) and existing `clipLinks.test.ts` still green; `MessageFiles.test.ts`: one image renders large with aspect ratio set, four images render a grid, video uses the player, audio renders `<audio>`, a pdf renders a file card with size, `Setup.exe` shows the warning, Download calls `save_attachment` with the name; `ImageViewer.test.ts`: arrows move, Escape closes, Save calls the command; `MessageItem.test.ts`: fallback content hidden, a caption shown.
- [ ] Step 2: FAIL → implement → PASS; commit `feat(attachments): show images, players and file cards (#57)`.

### Task 9: UI — search filters, then slice 1 verification and merge

**Files:** Modify `client/ui/src/lib/searchQuery.ts` (`has:` offers media and file; hint "Includes a link, a clip, a quote, media or a file") + its test.

- [ ] Step 1: test that `has:media` and `has:file` parse; FAIL → implement → PASS.
- [ ] Step 2: full verification: fmt --check, clippy -D warnings (workspace, all targets), server + client tests, `cargo xwin clippy -p pulse-client --target x86_64-pc-windows-msvc --all-targets -- -D warnings`, `just types-check`, `pnpm test`, `pnpm check`.
- [ ] Step 3: rebase on origin/main (renumber the migration if taken), re-run, merge to main, push.

---

## Slice 2 — big temporary sends (50 MB–2 GB), multipart with resume, 20 GB cap

### Task 10: Multipart in R2 and the fake S3

**Files:** Modify `server/src/media/r2.rs`, `server/src/testing/fake_s3.rs`.

**Produces (`R2`):** `create_multipart(key, content_type) -> AppResult<String>` (upload id), `part_url(key, upload_id, part: u16, ttl) -> (Url, String)`, `list_parts(key, upload_id) -> AppResult<Vec<(u16, String /*etag*/, u64 /*size*/)>>`, `complete_multipart(key, upload_id, parts: &[(u16, String)]) -> AppResult<()>`, `abort_multipart(key, upload_id) -> AppResult<()>` (already gone = done). rusty-s3 0.11 `actions::{CreateMultipartUpload, UploadPart, ListParts, CompleteMultipartUpload, AbortMultipartUpload}`. Protocol `MultipartInfo { part_size: u64, parts: u16 }`; `PART_SIZE = 16 MiB`.

Fake S3: `POST ?uploads` (XML with UploadId), `PUT ?partNumber&uploadId` (stores the part, answers an `ETag`), `GET ?uploadId` (ListParts XML), `POST ?uploadId` (complete: concatenates the listed parts in order; a missing or wrong ETag → 400 `InvalidPart`), `DELETE ?uploadId` (abort).

- [ ] Step 1: tests in `r2.rs`: `multipart_round_trip` (3 parts, list shows 2 after uploading 2, complete, HEAD size = total, GET bytes match), `abort_discards_parts`, `complete_with_a_wrong_etag_fails`; FAIL → implement → PASS; commit `feat(attachments): multipart uploads in R2 and the fake store (#57)`.

### Task 11: Server temp sends

**Files:** Modify `server/src/attachments/{mod.rs,routes.rs,sweep.rs}`, protocol (`AttachmentUpload.multipart`, `PartUrls`, `CompleteUploadRequest`).

Start with `temp_days: Some(1 | 7)` (anything else 400 "Temporary files expire in 1 or 7 days."): size must be > 50 MB and ≤ 2 GB (else 413 "Files can be up to 2 GB."); under the write lock, live temp bytes (`expires_at IS NOT NULL AND purged_at IS NULL`, pending included) + size > 20 GB → 409 `AppError::Conflict("This server's space for big files is full. Try again in a day or two.")`; key `temp/<key>`; `create_multipart`; the row stores `upload_id` and `expires_at = created_at + days`. At send, `bind` shifts it to `send time + (expires_at − created_at)`, so the days count from posting (no extra column).
Routes: `POST /attachment-uploads/{id}/parts {parts: Vec<u16>}` → `Vec<{part, url}>` (mine, pending, multipart; parts 1..=ceil(size/PART_SIZE)); `GET /attachment-uploads/{id}/parts` → done parts from `list_parts` (resume); `POST /attachment-uploads/{id}/complete` → `complete_multipart` with the listed parts, then HEAD size check (mismatch → abort, delete row, 409). `claim` refuses a temp row that isn't complete (409 "The upload didn't arrive. Try again.") — mark completion by clearing `upload_id`. Sweep: pending rows with `upload_id` abort the multipart instead of deleting an object; expired rows already handled (Task 4). `attach` reports `expires_at` and `expired` (`purged_at IS NOT NULL` or `expires_at < now`). URLs route: expired → `url: None`.

- [ ] Step 1: tests: `temp_send_needs_1_or_7_days`, `normal_size_with_temp_days_is_refused`, `over_2gb_is_refused`, `the_20gb_cap_refuses_new_big_sends` (seed rows totalling 19.9 GB; a 200 MB start is refused, a 50 MB normal file is not), `parts_are_signed_per_part`, `resume_lists_done_parts`, `complete_then_send_posts_with_expiry` (expires_at ≈ send + 1 day), `sending_an_incomplete_temp_upload_is_409`, `expired_temp_is_purged_and_shows_expired` (sweep at now + 2 days: object gone, row kept with `expired: true`, URL `None`, freed from the cap), `sweep_aborts_stale_multipart_uploads`; FAIL → implement → PASS; commit `feat(attachments): big temporary sends (#57)`.

### Task 12: Client temp sends

**Files:** Modify `client/src-tauri/src/attachments/send.rs`, `api.rs`; UI `ComposerFiles.svelte`, `FileCard.svelte`, `lib/attachments.ts`.

Core: files > 50 MB upload with multipart: request part URLs in batches of 8, PUT each 16 MiB slice (seek + take) with `put_file`-style streaming, 3 parts in parallel; progress summed; a failed part retries 3 times with backoff; a retry of the whole send first asks `GET …/parts` and skips done parts; then `complete`. The job's state (upload ids per staged file) survives a failed send so Retry resumes.
UI: when any staged file > 50 MB the chips row shows "Big files expire in [1 day | 7 days]" (segmented control, default 7 days) and the send passes `temp_days`; files > 2 GB show "Files can be up to 2 GB."; the cap message comes from the server error. File card: "expires in 6 days" / "expires tomorrow" / "expires in 5 hours" (`expiresLabel(expires_at, now)` in `lib/attachments.ts`), and **Expired** with Download hidden once `expired` or past `expires_at` (re-evaluated each minute).

- [ ] Step 1: tests: core `multipart_upload_resumes_after_a_failed_part` (mock store fails part 2 once), `retry_skips_parts_already_listed`; UI `expiresLabel` cases, picker appears only with a big file and sends `temp_days`, Expired card has no Download; FAIL → implement → PASS; full verification as Task 9, rebase, merge, push; commit `feat(attachments): send big files that expire (#57)`.

---

## Slice 3 — zip a dropped folder, `just storage`

### Task 13: Zip and send

Core: `zip_folder(path) -> Staged` writes `<folder name>.zip` into the staging dir (crate `zip` with deflate, already in the lockfile at 9.x; stream file by file; skip symlinks; refuse if the result would exceed 2 GB). UI: a dropped or pasted folder shows "Folders can't be sent as they are. [Zip and send]" in the chips row; the button stages the zip (spinner while zipping). Tests: `zips_a_folder_with_nested_files` (unzip and compare), UI shows the offer and stages the result.

### Task 14: `just storage`

Server subcommand `storage` (next to `clip-storage`): from the database, bytes and counts by kind — attachments (normal), temp (live), clips (videos + thumbnails), sounds and emoji (disk media rows by kind) — plus the R2 total and the estimated monthly cost (reuse `clips::format_storage_report`'s pricing). `justfile` recipe `storage` like `clips-check`. Test: the formatter on fixed numbers and one seeded database. Final commit says `Closes #57`.
