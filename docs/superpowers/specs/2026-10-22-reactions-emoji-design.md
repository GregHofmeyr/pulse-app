# Reactions, emoji and the media store: design

Date: 2026-10-22. Status: for the owner's review (builder check done, answers folded in).

Comes before: **quotes** (`2026-10-22-quotes-design.md`), which needs reactions, custom emoji and image uploads.
Later users of the media store: avatars and banners, badges' art if any, clips (R2).

## 1. Intent

Pulse has messages but no reactions and no emoji beyond what people type from their keyboard. The group lives on
reactions and custom emoji in Discord (friends' faces, inside jokes, animated ones). This push adds:

- **reactions** on messages;
- an **emoji picker** modelled on Discord's, used both for reactions and for typing in the composer;
- **custom emoji**, uploaded per server and usable anywhere by that server's members;
- the **media store**: the first time Pulse stores and serves uploaded files. Built once, reused by quotes (image
  quotes), avatars, banners and, later, clips.

The owner likes Discord's handling of all of this and wants it close to that. A reference screenshot of Discord's
picker is described in §3.1.

### Goals

- Reacting feels instant and looks like Discord.
- The picker finds an emoji quickly: frequently used first, search, a section per server.
- Custom emoji are cheap to store and load once per device.
- The media store is a single piece the next features plug into, with files on the server's disk or in R2 chosen per
  kind of file.

### Non-goals (this push)

- GIFs and stickers tabs, a GIF search, stickers. Bigger media gets its own rules later.
- Avatars and banners (they'll use this media store in their own small push). **My call**, to keep this push focused.
- Super reactions, reaction animations beyond a small pop, reaction notifications.
- Clips and R2 itself: the store supports an R2 location, but nothing in this push is put there (§5.2).

### Anti-goals

- **Slow or heavy:** emoji reloading on every launch, or the server serving the same file over and over.
- **Bloat:** keep to what Discord's picker does that the group uses.

## 2. Decisions made with the owner

| Topic | Decision |
|---|---|
| Reactions | Like Discord: a row of emoji + counts under a message; click to add/remove yours; "+" opens the picker. |
| Quotes channels | One reaction per person per quote (from the quotes design). **This push only leaves room for it** (§3.2); the quotes push builds it. |
| Picker | One picker for reactions and the composer. **Custom emoji at the top, the standard library underneath.** Modelled on Discord's (§3.1). |
| Custom emoji | Upload an image or GIF (animated allowed) with a name. **Stored per server; anyone in that server can use them everywhere**: other servers, DMs, groups (like Discord with Nitro). |
| Optimisation | Shrunk to fit 128×128 and converted to WebP (animated WebP for animations) on upload. |
| Storage | **One media store, two locations, chosen per kind of file.** Server disk for small, set-once or slow-growing files (emoji, avatars, banners, image quotes); **R2** for big or fast-growing ones (clips, later). Switching a kind is configuration. |
| Bandwidth | Files have content-based addresses and never change, so each device downloads a file once and caches it. |

## 3. Reactions and the picker

### 3.1 The picker (from the owner's Discord reference)

- **Search** box at the top (matches names and keywords; `:name:` also works).
- **Left rail:** "Frequently used" (clock icon), then **one icon per server** that has custom emoji (the server's
  icon), then the standard categories (smileys, people, nature, food, activities, travel, objects, symbols, flags).
  Clicking jumps to that section. The rail's scrollbar is thin and themed (the owner dislikes the white one in his
  Discord client).
- **Main grid:** collapsible sections ("Frequently used ▾", "<server name> ▾", then the standard categories), emoji at a
  comfortable size, hover highlight.
- **Footer:** the hovered emoji larger, with its `:name:`.
- **My calls:**
  - Frequently used is per user, kept on the server so it follows them between devices, and ranked by use with a
    gentle decay (recent use counts more). About 3 rows.
  - The standard library: Unicode emoji with a bundled set of images (for example Twemoji or Noto), so emoji look the
    same on Windows and Linux. Skin tones: one setting in the picker, like Discord.
  - Custom emoji from servers you're **not** a member of don't show in your picker.
  - Keyboard: arrows move, Enter picks, Esc closes; typing goes to search.

### 3.2 Reactions on messages

- The row under a message: each emoji with its count; yours highlighted. Click to toggle yours.
- Hovering a reaction shows who reacted (names, "and N others" past about 10).
- "+" at the end of the row, and a smiley button in the message's hover bar, open the picker.
- **My calls:**
  - Up to 20 different emoji on one message.
  - Reactions update live for everyone who can see the message, through the existing gateway and audience rules.
  - A custom emoji that's deleted later stays on old messages as its `:name:` text in a muted chip (the image is
    gone); counts are kept.
  - **Room for quotes (my call, after the builder check):** reactions go through one server function with a per-channel
    "reaction mode" (normal, or one per person). Every channel is normal in this push; the quotes push adds the
    `quotes` channel kind (its migration) and switches its channels to one-per-person. No quotes code here.

### 3.3 Emoji in messages

- The composer has an emoji button opening the same picker, and `:` + two letters opens an autocomplete (like
  @mentions) for custom and standard emoji.
- Custom emoji are stored in the message text as a token (for example `<:name:id>`) and drawn as images; standard
  emoji stay plain Unicode in the text.
- A message that is only emoji (up to about 27) shows them larger, like Discord. **My call.**
- Someone who can't use a custom emoji (not in its server) still **sees** it in messages; they just can't pick it.

## 4. Custom emoji

### 4.1 Adding and managing

- In server settings, an **Emoji** page: a grid of the server's emoji with name, who added it, and when.
- **Add emoji:** pick a PNG, JPEG, WebP or GIF (animated GIF/WebP allowed). Name it (defaults from the file name).
- **My calls:**
  - Anyone in the server can add, rename and delete emoji (flat rules, like the rest of Pulse). Deleting asks "are you
    sure?".
  - Names: 2–32 characters, letters, digits and underscores; unique within the server. Two servers can both have
    `:pog:`; the picker shows them under their own server, and autocomplete shows the server name next to each.
  - Up to **100 emoji per server** (50 still + 50 animated is Discord's free limit; ours is one pool, raise later).
  - Upload limit **2 MB** before processing (a big GIF shrinks a lot).

### 4.2 Processing

- On upload the server checks the real file type (not the extension), then:
  - decodes it, fits it within **128×128** keeping its shape (transparent padding where needed);
  - still images → **WebP**; animated → **animated WebP**, keeping frame timing; very long or huge animations are
    refused with a clear message (**my call:** at most 300 frames after processing, at most 512 KB output);
  - strips metadata;
  - for animated files, also stores a **still first-frame** WebP (added 2026-10-22 from the profiles design: an
    `<img>` can't pause an animation, and lists and reduced motion need a still);
  - the original is not kept.
- The client shows a preview of the processed result before saving (so a badly cropped one is obvious).

## 5. The media store

### 5.1 How it works

- Every stored file has a **content address** (a hash of its bytes) and a **kind** (emoji, image quote, avatar,
  banner, clip…). The same bytes uploaded twice are stored once.
- The database keeps a row per file (hash, kind, size, content type, dimensions, frames, location, who uploaded it,
  when) and what uses it (an emoji, a quote line…). A file nothing uses any more is deleted by a periodic cleanup.
- Files are served from a URL that includes the hash. Because the bytes behind a URL never change, responses are sent
  with long-lived, immutable cache headers, and each device downloads a file **once**. The client also keeps its own
  on-disk cache so a restart doesn't refetch.
- **Only group members can fetch files.** The builder picks the mechanism (for example short-lived signed URLs, or a
  session cookie the webview sends, or fetching through the Tauri core); the hash alone must not be enough. Media must
  not be reachable from the open internet without a login.

### 5.2 Locations

- **Server disk** (under the existing `/srv/pulse/data`): emoji and image quotes in this push; avatars and banners
  later. These are small and set once or grow slowly.
- **R2: not built in this push (my call, after the builder check).** The store has a `location` on every file and a
  per-kind setting, with the server disk as the only location that exists. The R2 location (client, credentials,
  signed reads, the copy command) is built by the first feature that needs it, clips.
- **Backups:** the nightly backup (`docs/superpowers/specs/2026-10-09-deploy-design.md` §5) currently copies only the
  database. It must also include the media directory (encrypted the same way), or a lost VM loses every emoji.
- **Disk guard:** the server refuses uploads with a clear error when the disk is nearly full (**my call:** less than
  2 GB free), and logs a warning well before that.

### 5.3 Limits and abuse

- Upload rate limit per user, in the existing `limits.rs` style (**my call:** 30 uploads per hour).
- Size and type checks as in §4.2; anything that fails to decode is refused.

## 6. Data and events (for the builder)

Names are suggestions.

- `reactions(message_id, user_id, emoji, created_at)` already exists (0001). `emoji` holds either a Unicode emoji or a
  custom emoji reference (`custom:<id>`).
- `custom_emoji(id, server_id, name, media_hash, animated, added_by, created_at)`, unique `(server_id, name)`.
- `media(hash, kind, location, content_type, size, width, height, frames, uploaded_by, created_at)` and a usage link
  (or reference counting) for cleanup.
- `emoji_usage(user_id, emoji, score, last_used_at)` for Frequently used.
- `Message` carries its reactions summary (emoji, count, whether I reacted) so a channel loads in one request.
- Gateway events: reaction added/removed (with the new count); custom emoji added/renamed/removed (server audience).
- Every rule is enforced by the server.

## 7. Tests

- Server units: reaction toggle, 20-emoji cap, the one-per-person reaction mode (through a test-only channel setting), audience (DM reactions only reach DM
  members); emoji name rules and per-server uniqueness; upload validation (wrong type, too big, corrupt, huge
  animation), processing output size and type; dedupe by hash; cleanup of unused files; cache headers; a file can't be
  fetched without a valid login; disk guard.
- Gateway: reaction and emoji events reach the right people live.
- UI: picker (search, rail jumps, frequently used, keyboard), reaction row (toggle, tooltip, overflow), composer `:`
  autocomplete, big-emoji messages, a deleted custom emoji's fallback chip.
- Backup: a restore drill includes the media directory.
- Manual: upload a dozen real emoji (some animated) from the group's Discord and react with them on Windows.

## 8. Left to the builder

- The image library used for decoding and WebP encoding (it must handle animated GIF → animated WebP on the ARM
  server), and the bundled emoji set.
- The media authorisation mechanism (§5.1), within the rule that a login is required. Note: the server only accepts
  `Authorization: Bearer` today, the webview's CSP is `img-src 'self' data:`, and an `<img>` can't send a header, so
  this needs real design (for example a Tauri custom protocol that fetches through the core, or signed URLs plus a CSP
  change).
- Exact picker sizes and animations.
