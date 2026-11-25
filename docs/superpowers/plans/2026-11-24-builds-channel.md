# Builds Channel Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** a `builds` channel kind that stores each character build as one tidy item (title, game, link, screenshots,
notes) with comments, by-game browsing, archive-not-delete, and pin/peek/pop-out quick access.

**Architecture:** a build is a message of a builds channel plus one `builds` row keyed by `message_id`, exactly as a
clip is (`server/src/clips/`): author, unread, reactions and attachments come with the message; comments are thread
replies (`thread_root_id`); the message's `content` is a plain-text rendering of the build (as `quotes::render_content`
does for quotes), so the existing FTS triggers index it with no trigger changes. The client adds a browse view, a build
page and quick access on top of the existing feeds, thread, GamePicker, LinkEmbed and ImageViewer code; the pop-out is a
second app window behind an IPC gate.

**Tech Stack:** Rust (axum, sqlx/SQLite, ts-rs), Tauri 2 core (reqwest), Svelte 5 + vitest.

**Spec:** `docs/superpowers/specs/2026-11-24-builds-channel-design.md` (§9 and its "my call" notes are settled).

## Global Constraints

- Limits (server-enforced, §6.2): title 1–120 characters; notes ≤ 4000 (`messages::MAX_LEN`); at most 10 screenshots;
  link ≤ 2048 characters, `http`/`https` only.
- Only a title is required; a link, an image or plain text alone makes a valid build (§2, §4.3).
- Author-only: edit, change game, archive/restore. Server owner (`servers.created_by`) also archives, restores and
  deletes any build. Delete = author or owner (§5.1).
- Archived builds: hidden by default, greyed with an `ARCHIVED` badge when shown, still searchable, openable and
  commentable; archiving never deletes (§1 anti-goals, §5.2).
- Sort is newest first by when saved (message id); an update never reorders (§3.2).
- Game shown as a small-caps caption above the title, never a pill; no filter pills (§2).
- Copy: "+ Add build", "Gallery" / "List", "Show archived", "Archive old builds", "Archive matching builds…",
  "QUICK DROP", "ARCHIVED", "Untitled build", "guessed", "All games", "Actions", "Pin to side", "Pop out", "Edit",
  "Archive", "author", "Mark as sorted".
- Not in phase 1: Copy link, moderating others' comments, an admin role, sort options, tags, version history,
  per-game fields (§8). Task 0 opens their issues.
- Repo rules (`CLAUDE.md`): every deferral gets a GitHub issue before the deferring work merges; commits reference
  their issue (`#N`), the last commit of a phase says `Closes #N`; test people are alex, sam, kim and jo only.
- Migrations are immutable; the new one is `server/migrations/0082_builds.sql`.
- Protocol types change → `just gen-types` and commit `client/ui/src/lib/protocol/*` in the same commit.
- Local runs: `CARGO_BUILD_JOBS=2 nice -n 10` and targeted tests only; the full suite runs on CI.

## Review Focus

1. **An older client meets a builds channel.** `ChannelKind` has no catch-all, so a protocol-4 client fails to parse a
   Ready that holds a builds channel. Expect: it is told to update, never broken. → Task 13 test.
2. **A link that doesn't unfurl** (planner page with no title, timeout, 404). Expect: the build is saved at once and
   keeps the link (shortened) as its title. → Task 3 test.
3. **The author retitles a quick drop while its unfurl is in flight.** Expect: the fetched title never overwrites a
   title the person typed. → Task 3 test.
4. **A fetched page title over 120 characters.** Expect: truncated to 120 with "…", never a refused save. → Task 3 test.
5. **A pinned or popped-out build is deleted, or you lose access (left the server).** Expect: the side panel says it's
   gone and unpins; the pop-out closes. → Task 20 and Task 25 tests.

## Phases and parallelism

| Phase | Tasks | Depends on |
|---|---|---|
| 1 Server and storage | 0–7 | – |
| 2 Client browse views | 8–13 | 1 |
| 3 Build page and adding | 14–19 | 1; 15, 18, 19 also need 2 |
| 4 Quick access | 20–22 | 2, 3 |
| 5 Pop-out window | 23–25 | 3 |

Phases 2 and 3 should go out in the same client release (phase 2 alone browses a channel nobody can add to yet).
Parallel: inside phase 1, Tasks 3, 5, 6 and 7 can run side by side after Task 2. Phase 3's Tasks 14 (ThreadChat
extract), 16 (video) and 17 (core uploads) can run alongside phase 2. Phases 4 and 5 can run in parallel once phase 3
merges. Each phase is its own PR to `main`.

---

## Phase 1: Server and storage

### Task 0: Issues

- [ ] **Step 1:** Open a `feature` issue per phase ("Builds channel: server", "…: browse", "…: build page and adding",
  "…: quick access", "…: pop-out"), labels `area: chat`, `size: M|L`, `queued`, body pointing at the spec. Written below
  as `#P1`…`#P5`; substitute the real numbers in commits.
- [ ] **Step 2:** Open `parked` issues for each §8 deferral (Copy link, moderating comments, admin role, sort options,
  tags, version history, per-game fields) and for this plan's own deferral: inline video in the pop-out (Task 25).

### Task 1: Protocol, migration 0082 and the channel kind

**Files:**
- Create: `server/migrations/0082_builds.sql`, `server/tests/builds.rs`
- Modify: `protocol/src/rest.rs` (`ChannelKind`, `Message`, new types), `protocol/src/gateway.rs`, `protocol/src/lib.rs`
  (`PROTOCOL_VERSION`), `server/src/pins/mod.rs` (`line_noun`), `server/src/forwards.rs:369`,
  `server/src/polls/routes.rs:37`, `server/src/scheduled.rs:135`, `server/src/wheels/routes.rs:252`,
  `server/src/plans/routes.rs:81`, `server/src/messages/routes.rs:414-419`, every `Message { … }` literal
  (`grep -rn "wheel: None"`: 7 sites)

**Interfaces:**
- Produces (protocol):
  - `ChannelKind::Builds` (`"builds"`); `is_server_chat()` includes it.
  - `Message.build: Option<Box<Build>>` with `#[serde(default, skip_serializing_if = "Option::is_none")]`, `#[ts(optional)]`.
  - `Build { title: String, title_auto: bool, game_id: Option<GameId>, game_guessed: bool, link: Option<BuildLink>,
    notes: Option<String>, quick_drop: bool, updated_at: Option<String>, archived_at: Option<String>,
    archived_by: Option<UserId>, reply_count: u32, last_reply_id: Option<MessageId>, replies_seen: Option<MessageId> }`
    (screenshots are `Message.attachments`).
  - `BuildLink { url: String, title: Option<String>, thumb: Option<EmbedThumb>, provider: EmbedProvider, provider_id: Option<String> }`
  - `CreateBuildRequest { title: Option<String>, game_id: Option<GameId>, link_url: Option<String>, notes: Option<String>, attachments: Vec<AttachmentId>, nonce: Option<String> }`
  - `UpdateBuildRequest { title: String, game_id: Option<GameId>, link_url: Option<String>, notes: Option<String>, attachments: Vec<AttachmentId>, mark_sorted: bool }` (`attachments` = the final order: kept ids and new uploads)
    - follow-up: `game: Option<BuildGameEdit>` (`"unchanged"` | `{"set": id}` | `"clear"`); when present it replaces `game_id`, and `"unchanged"` keeps a guessed game a guess (and the quick drop)
  - `SetBuildArchivedRequest { archived: bool }`
  - `BulkArchiveRequest { game_id: Option<GameId>, before: String, dry_run: bool }` → `BulkArchiveResult { builds: Vec<BuildBrief>, archived: u32 }`, `BuildBrief { message_id: MessageId, title: String, game_id: Option<GameId>, created_at: String }`
  - `BuildFacets { total: u32, games: Vec<GameCount> }`
  - `Event::BuildsArchived { channel_id: ChannelId, message_ids: Vec<MessageId>, archived_at: String, archived_by: UserId }` (audience: the channel, in the `channel_id` match arm)
  - `PROTOCOL_VERSION = 5` with the doc line "5: builds channels (#P1); a version-4 client can't parse `ChannelKind::Builds`".

Migration 0082 (one file):
- The `channels` rebuild exactly as 0009 (guard on `pragma_foreign_keys`, `channels_new`, copy, drop, rename), with
  `kind IN ('text','voice','dm','group','quotes','clips','builds')` and **every column `channels` has now**: `id,
  server_id, kind, name, position, created_at, reaction_mode, deleted_at, deleted_by, icon_hash, category_id (REFERENCES
  categories(id) ON DELETE SET NULL), bitrate, archived_at`; recreate `channels_server`, `channels_deleted` (partial),
  `channels_category`. The FK-check guard covers `channels`, `categories` and every table with `REFERENCES channels`
  (24 today: re-run `grep -ln "REFERENCES channels" server/migrations` when writing it).
- `builds (message_id TEXT PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE, channel_id TEXT NOT NULL REFERENCES
  channels(id) ON DELETE CASCADE, title TEXT NOT NULL, title_auto INTEGER NOT NULL DEFAULT 0, game_id TEXT REFERENCES
  games(id), game_guessed INTEGER NOT NULL DEFAULT 0, link_url TEXT, link_title TEXT, link_thumb TEXT REFERENCES
  media(hash), link_provider TEXT, link_provider_id TEXT, notes TEXT, quick_drop INTEGER NOT NULL, updated_at TEXT,
  archived_at TEXT, archived_by TEXT REFERENCES users(id), CHECK ((archived_at IS NULL) = (archived_by IS NULL))) STRICT`.
  `channel_id` is denormalised so the spec's indexes exist: `builds_browse (channel_id, archived_at, message_id)`,
  `builds_game (channel_id, game_id, message_id)`. Search needs none (it rides `search_docs`).
- No `categories.default_kind` change: a builds channel goes in the category it's created in.

- [ ] **Step 1: Write the failing tests** in `server/tests/builds.rs` (helpers copied from `tests/clips.rs`:
  `hello`, `create_clips` → `create_builds`):
  - `creates_a_builds_channel`: `POST /servers/{id}/channels {kind:"builds"}` → 200, `ch.kind == ChannelKind::Builds`,
    `reaction_mode == "normal"`, sam (member) gets `ChannelCreated`.
  - `plain_messages_are_refused_in_a_builds_channel`: `POST /channels/{id}/messages` → 400 "only builds can be posted here".
  - `polls_plans_wheels_and_forwards_are_refused_in_a_builds_channel`: each → 400.
  - `migration_0082_keeps_every_channel_row`: as `clips::migration_keeps_every_channel_row`, migrate to 81, insert a
    channel of each kind with `deleted_at`, `category_id`, `bitrate`, `archived_at`, `icon_hash` set, run 82, assert
    every column of every row is unchanged and `PRAGMA foreign_key_check` is empty.
  - `migration_0082_refuses_to_run_with_foreign_keys_on`: as the 0009 one.
- [ ] **Step 2:** `cargo test -p pulse-server --test builds` → FAIL (no `Builds` variant).
- [ ] **Step 3:** Add the protocol types, migration and kind lists above (`ChannelKind::Builds => None` in `line_noun`;
  add `Builds` to the five `matches!(… Quotes | Clips)` refusals; the send refusal message "only builds can be posted
  here"; `build: None` in every literal).
- [ ] **Step 4:** Tests pass; `just gen-types`; `cargo test -p pulse-server --test clips --test categories` still pass.
- [ ] **Step 5: Commit** `git add protocol server/migrations/0082_builds.sql server/src server/tests/builds.rs client/ui/src/lib/protocol`
  → `git commit -m "Builds: the builds channel kind and its storage (#P1)"`

### Task 2: Create, get and list builds

**Files:**
- Create: `server/src/builds/mod.rs` (attach, render, rules), `server/src/builds/routes.rs`
- Modify: `server/src/lib.rs` (router), `server/src/messages/routes.rs` (`fill` and `load` call `builds::attach`),
  `server/src/servers/routes.rs` (`is_owner_in`)
- Test: `server/tests/builds.rs`

**Interfaces:**
- Produces:
  - `pub const TITLE_MAX: usize = 120; NOTES_MAX = crate::messages::MAX_LEN; MAX_SCREENSHOTS: usize = 10; LINK_MAX: usize = 2048; PAGE: u32 = 30; UNTITLED: &str = "Untitled build";`
  - `pub async fn attach(db: &SqlitePool, me: Option<UserId>, msgs: &mut [Message]) -> AppResult<()>` (as `clips::attach`, thread summary via `messages::threads::summaries`)
  - `pub fn render_content(title: &str, game: Option<&str>, link_title: Option<&str>, notes: Option<&str>) -> String` — the non-empty parts joined by `\n`
  - `pub fn clean_link(raw: &str) -> AppResult<String>` (trim, ≤ 2048, parses, scheme http/https)
  - `pub(crate) async fn is_owner_in(db: impl SqliteExecutor, server: ServerId, me: UserId) -> AppResult<bool>` in `servers/routes.rs` (reads `servers.created_by`)
  - Routes: `POST /channels/{id}/builds` (`CreateBuildRequest` → `Message`), `GET /builds/{message_id}` → `Message`,
    `GET /channels/{id}/builds?game=&archived=&q=&before=&limit=` → `Vec<Message>` (`q` lands in Task 6).

Create rules: channel is `Builds`, live and open (`access::require_open`, `channel_live_in`, `open_in` in the write),
caller a server member, `media::writes_ok`, `nonce::check` + replay as clips' `prior`/`replay`. Title: trimmed request
title; else, with a link and no title, the link shortened to host + path (≤ 120) with `title_auto = 1`; else with
screenshots only, `UNTITLED` with `title_auto = 1`; nothing at all → 400 "A build needs a title, a link or a
screenshot." Game must be `clips::games::in_server`. `quick_drop = game_id.is_none() && notes.is_none()`. Screenshots:
`attachments::claim` then `attachments::bind` in the write; any non-image → 400 "Screenshots must be images.".
`mentions: vec![]` (notes don't ping). Then `reads::advance` for the author, commit, `MessageCreated`,
`ReadStateUpdated` as clips' `confirm` does. No `embeds::spawn_unfurl` (the link's preview is Task 3's).

- [ ] **Step 1: Write the failing tests:**
  - `creates_a_build_with_a_title_only`: alex posts `{title:"Raven crit"}` → `m.build.title == "Raven crit"`,
    `quick_drop == true`, `m.content == "Raven crit"`, sam's socket gets `MessageCreated` with `build`.
  - `creates_a_build_from_a_link_only`: `{link_url:"https://youtu.be/abc12345678"}` → title `"youtu.be/abc12345678"`, `title_auto`.
  - `creates_a_build_from_an_image_only`: one uploaded WebP (helpers from `tests/attachments_send.rs`) → title
    `"Untitled build"`, `m.attachments.len() == 1`.
  - `a_build_with_game_and_notes_is_not_a_quick_drop`.
  - `build_inputs_are_checked`: 121-char title, 4001-char notes, 11 screenshots, `ftp://` link, 2049-char link, a game
    from another server, a PDF attachment → each 400; empty body → 400 "A build needs a title, a link or a screenshot."
  - `non_members_cant_add_or_see_builds`: kim (not in the server) → create, `GET /builds/{id}` and list are each
    refused with the status `access::channel_for` gives a non-member on the clips routes (assert that same code).
  - `lists_builds_newest_first_by_game_without_archived`: three builds (two games), list → newest first; `?game=` filters.
  - `a_build_counts_as_unread_for_others`: sam's read state for the channel shows 1 unread after alex's build.
- [ ] **Step 2:** `cargo test -p pulse-server --test builds` → FAIL (404 on the route).
- [ ] **Step 3:** Implement the functions and routes above; `fill`/`load` call `crate::builds::attach` after `clips::attach`.
- [ ] **Step 4:** Tests pass.
- [ ] **Step 5: Commit** "Builds: create, get and list (#P1)".

### Task 3: The link's title, thumbnail and game guess

**Files:**
- Create: `server/src/builds/unfurl.rs`
- Modify: `server/src/builds/routes.rs` (spawn after create and after a link change), `server/src/media/mod.rs:362-390`
  (`cleanup` keeps `builds.link_thumb`, both the direct list and the still branch)
- Test: `server/tests/builds_unfurl.rs` (fixtures as `tests/embeds_unfurl.rs`)

**Interfaces:**
- Consumes: `Unfurler::unfurl(&self, db, media_dir, by, thumbs, url) -> Option<Unfurled>` (`s.embeds`), `embeds::keep_thumb`, `s.limits.unfurl_user`, `s.limits.link_thumb_user`.
- Produces:
  - `pub fn spawn_fill(s: &AppState, message_id: MessageId, author: UserId, link: String)`
  - `pub fn guess_game(title: &str, games: &[(GameId, String)]) -> Option<GameId>` — the longest registry name whose
    `quotes::name_key` is contained in the title's `name_key` on word boundaries.
  - `pub fn fit_title(s: &str) -> String` — trimmed, ≤ 120 characters, cut with "…".

`spawn_fill` writes `link_title`, `link_thumb`, `link_provider`, `link_provider_id` only while the row's `link_url` is
still `link` and the message live (guarded UPDATE); when `title_auto = 1` and a title came back, `title =
fit_title(title)` and `title_auto = 0`; when `game_id IS NULL`, the guess with `game_guessed = 1` (stays a quick drop).
It rewrites `content` with `render_content` and publishes `MessageUpdated` under `s.hub.in_order()`.

- [ ] **Step 1: Write the failing tests:**
  - `a_youtube_link_fills_title_thumb_and_provider`: fixture oEmbed title "Raven Crit Build - The First Descendant" →
    after the `MessageUpdated`, title is it, `link.provider == Youtube`, `link.provider_id == Some("abc12345678")`, `link.thumb.is_some()`.
  - `the_game_is_guessed_from_the_link_title`: registry has "The First Descendant" → `game_id` set, `game_guessed`, `quick_drop` still true.
  - `an_unreachable_link_keeps_the_link_as_title` *(Review Focus 2)*: fetch 404 → no update; title stays `"example.com/planner"`.
  - `unfurl_never_overwrites_a_title_the_author_typed` *(Review Focus 3)*: alex PUTs a title while the fixture server
    holds the response → final title is alex's.
  - `a_long_page_title_is_cut_to_120` *(Review Focus 4)*: 300-char title → 120 chars ending "…".
  - `a_builds_link_thumb_survives_media_cleanup`: `media::cleanup` with zero grace keeps the thumb.
  - unit: `guess_game_takes_the_longest_whole_word_match` ("Far Far West" over "West"; "Warframe" not in "Warframes").
- [ ] **Step 2:** Run → FAIL.  **Step 3:** Implement.  **Step 4:** Pass.
- [ ] **Step 5: Commit** "Builds: fetch the link's title and thumbnail, guess the game (#P1)".

### Task 4: Edit (author only)

**Files:**
- Modify: `server/src/builds/routes.rs` (`PUT /builds/{message_id}`), `server/src/attachments/mod.rs`,
  `server/src/messages/routes.rs` (`edit` and `remove` refuse builds)
- Test: `server/tests/builds.rs`

**Interfaces:**
- Produces:
  - `pub async fn set_message_files(tx: &mut SqliteConnection, me: UserId, message: MessageId, ordered: &[AttachmentId], new: &[AttachmentId]) -> AppResult<Vec<Stored>>` — binds `new`, renumbers `position` to `ordered`, deletes the rows of the message's files not in `ordered`, returns their objects for `remove_objects` after the commit.
  - `PUT /builds/{id}` (`UpdateBuildRequest` → `Message`); publishes `MessageUpdated`.

Rules: `own_message` (author only, else 404 as clips' edit), `require_open`; same input checks as create; `title_auto =
0`; an explicit `game_id` clears `game_guessed`; `quick_drop` only ever goes true → false (game set, notes non-empty,
or `mark_sorted`); `updated_at = now()`; `content` re-rendered; a changed link clears the `link_*` fields and calls
`spawn_fill`. `messages::routes::edit` → 400 "builds are edited with their own route"; `remove` → 400 "builds are
deleted with their own route".

- [ ] **Step 1: Write the failing tests:**
  - `only_the_author_edits_a_build`: sam's PUT → 404; alex's → 200, `updated_at` set, `MessageUpdated` reaches sam.
  - `an_edit_keeps_the_builds_place_in_the_list`: edit the oldest of three → still last.
  - `edit_reorders_adds_and_removes_screenshots`: [a,b] → [b,c] → attachments order `[b,c]`, `a`'s row gone and its object deleted.
  - `quick_drop_ends_with_a_game_notes_or_mark_sorted_and_never_returns`.
  - `clearing_a_guessed_game_sticks`: PUT `game_id: null` → `game_id` none, `game_guessed` false.
  - `builds_refuse_the_message_edit_and_delete_routes`.
- [ ] **Step 2–4:** Run → FAIL, implement, pass.
- [ ] **Step 5: Commit** "Builds: edit by the author (#P1)".

### Task 5: Archive, restore, bulk archive, delete and facets

**Files:**
- Create: `server/src/builds/archive.rs`
- Modify: `server/src/builds/routes.rs`
- Test: `server/tests/builds_archive.rs`

**Interfaces:**
- Produces routes:
  - `PUT /builds/{id}/archived` (`SetBuildArchivedRequest` → `Message`): author or owner; sets/clears
    `archived_at`/`archived_by`; `MessageUpdated`. Refused in an archived channel (`require_open`).
  - `POST /channels/{id}/builds/archive` (`BulkArchiveRequest` → `BulkArchiveResult`): matches live, unarchived builds
    with `m.created_at < before` and, when given, `game_id`; for a non-owner only their own. `dry_run` lists them
    (newest first, at most 500; more → 400 "Narrow it down: over 500 builds match."); otherwise one UPDATE in one
    write, then one `Event::BuildsArchived`.
  - `DELETE /builds/{id}` (204): author or owner; soft-deletes the message as `messages::routes::remove` does (content
    cleared, reactions gone, `threads::soft_delete_replies`, `embeds::on_delete`, `history::on_delete`,
    `pins::on_delete`), deletes the `builds` row, removes the screenshots' objects after commit
    (`attachments::stored_of_message` + `remove_objects`); publishes `MessageDeleted` plus the replies' events in one
    `publish_all`.
  - `GET /channels/{id}/builds/facets?archived=` → `BuildFacets` (counts per game, plus `total` incl. no game).

- [ ] **Step 1: Write the failing tests:**
  - `only_the_author_or_owner_archives`: alex owns the server; sam's build: kim 404/403, sam 200, alex 200; restore clears both fields.
  - `archived_builds_are_hidden_unless_asked`: list default excludes, `?archived=true` includes, `GET /builds/{id}` still works.
  - `commenting_on_an_archived_build_works`.
  - `bulk_archive_by_game_with_a_cutoff`: dry run lists only that game's builds before the cutoff; the real run archives
    exactly those, returns `archived == n`, sockets get one `BuildsArchived` with those ids.
  - `bulk_archive_for_all_games_only_touches_allowed_builds`: sam's bulk covers only sam's; alex (owner) covers all.
  - `owner_deletes_any_build_with_its_screenshots_and_comments`: replies soft-deleted, objects gone, `MessageDeleted` out.
  - `facets_count_games_and_respect_archived`.
- [ ] **Step 2–4:** Run → FAIL, implement, pass.
- [ ] **Step 5: Commit** "Builds: archive, bulk archive, delete and facets (#P1)".

### Task 6: Search

**Files:**
- Modify: `server/src/builds/routes.rs` (`q` on the list; `GET /builds/search`)
- Test: `server/tests/builds.rs`

**Interfaces:**
- Consumes: `search::text::{terms, fts_query}`, `message_search`/`search_docs` (channel-scoped via `search_docs_channel`),
  `access::readable_server_channels` / `readable_channel_ids_everywhere`.
- Produces: `GET /builds/search?q=&game=<name_key>&limit=` → `Vec<Message>` across the caller's readable builds
  channels: unarchived first, then archived, each newest first; `limit` ≤ 20. `game` matches `games.name_key` (one name
  across servers).

- [ ] **Step 1: Write the failing tests:**
  - `channel_search_finds_title_notes_game_and_link_title`: four builds, one term each → each found by `?q=`.
  - `search_everywhere_lists_current_before_archived`.
  - `search_everywhere_filters_by_game_key_across_servers`: "The First Descendant" in two servers → both with `game=the first descendant`.
  - `search_never_shows_another_servers_builds`: kim sees none of alex's server.
- [ ] **Step 2–4:** Run → FAIL, implement, pass.
- [ ] **Step 5: Commit** "Builds: search (#P1)".

### Task 7: Comments in the inbox Threads list

**Files:**
- Modify: `server/src/messages/threads.rs` (`is_live_root`, `visible_root`), `server/src/inbox/threads.rs` (`WATCHED`)
- Test: `server/tests/inbox_threads.rs`

**Interfaces:**
- Produces: `pub(crate) const THREAD_ROOT: &str` in `messages/threads.rs` — the `(EXISTS quotes OR EXISTS clips OR
  EXISTS builds)` fragment over alias `m`, used by all three queries (WATCHED aliases `r`: take the alias as a
  parameter, `fn thread_root(alias: &str) -> String`).

- [ ] **Step 1: Write the failing tests** (a builds fixture beside the quotes one in `inbox_threads.rs`):
  - `a_comment_on_a_build_reaches_its_author_and_earlier_commenters`: alex's build; sam comments → alex's Threads has
    it; kim comments → alex and sam have it.
  - `your_own_comment_doesnt_notify_you`: sam comments → sam's Threads lacks it.
  - `comments_dont_mark_the_builds_channel_unread`.
- [ ] **Step 2–4:** Run → FAIL, implement, pass. Run `--test inbox_threads --test quotes --test clips` (no regressions).
- [ ] **Step 5: Commit** "Builds: comments are threads, in the inbox (Closes #P1)".

---

## Phase 2: Client browse views

### Task 8: Core commands for browsing

**Files:**
- Create: `client/src-tauri/src/builds.rs` (`impl Api` block + `#[tauri::command]`s)
- Modify: `client/src-tauri/src/lib.rs` (module, `generate_handler!`), `client/ui/src/lib/tauri.ts`

**Interfaces:**
- Produces commands (and `api.*` in `tauri.ts`): `list_builds(channel_id, game, archived, q, before) -> Vec<Message>`,
  `get_build(message_id) -> Message`, `build_facets(channel_id, archived) -> BuildFacets`,
  `set_build_archived(message_id, archived) -> Message`, `bulk_archive_builds(channel_id, body: BulkArchiveRequest) ->
  BulkArchiveResult`, `delete_build(message_id) -> ()`, `search_builds(q, game) -> Vec<Message>`.
  TS: `listBuilds`, `getBuild`, `buildFacets`, `setBuildArchived`, `bulkArchiveBuilds`, `deleteBuild`, `searchBuilds`.

- [ ] **Step 1:** Tests in `builds.rs` against the mock server the `api.rs` tests use: `list_builds_sends_its_filters`
  (query has `game`, `archived=true`, `q`, `before`), `bulk_archive_posts_its_body`, `delete_build_is_a_delete`.
- [ ] **Step 2–4:** `cargo test -p pulse-client --lib builds::` FAIL → implement → PASS.
- [ ] **Step 5: Commit** "Builds: core commands for browsing (#P2)".

### Task 9: State: build feeds and events

**Files:**
- Create: `client/ui/src/lib/builds.ts`, `client/ui/src/lib/builds.test.ts`
- Modify: `client/ui/src/lib/state.ts` (`AppState.buildFeeds`, `buildFacets`; `applyEvent` hooks; `withThread` and
  `loadedRoot` know `m.build`; `applyReady` clears both)

**Interfaces:**
- Produces:
  - `type BuildFilter = { game?: string; archived?: boolean; q?: string }`
  - `buildFeedKey(channelId: string, f: BuildFilter): string`
  - `openBuildFeed(s, channelId, f): AppState`, `loadBuildPage(s, key, page: Message[]): AppState`, `buildFeedFailed(s, key): AppState`
  - `buildsOnEvent(s: AppState, e: Event): AppState` — `MessageCreated`/`MessageUpdated` with `build` insert/replace/drop
    per feed filter (as `clipCreated`/`clipUpdated`, keeping `replies_seen` and reactions), `MessageDeleted` drops,
    `BuildsArchived` sets `archived_at`/`archived_by` and drops from feeds without `archived`.
  - `matchesBuildFilter(m: Message, f: BuildFilter): boolean` (game and archived only; `q` feeds are refetched)
  - `isQuickDrop(m)`, `isArchived(m)`, `buildThumb(m): { kind: 'shot' | 'video' | 'none'; … }` (first screenshot, else link thumb, else none)

- [ ] **Step 1: Write failing tests** in `builds.test.ts`: `a new build joins only feeds it matches`, `an update keeps
  its place and its seen point`, `archiving drops it from the default feed and greys it in the archived one`,
  `BuildsArchived updates every listed build`, `a deleted build leaves every feed`, `thumb prefers a screenshot then the video`.
- [ ] **Step 2–4:** `cd client/ui && pnpm vitest run src/lib/builds.test.ts` FAIL → implement → PASS; `pnpm check`.
- [ ] **Step 5: Commit** "Builds: feeds in the client state (#P2)".

### Task 10: Creating a builds channel and opening it

**Files:**
- Create: `client/ui/src/components/BuildsChannel.svelte` (shell: header + empty state for now)
- Modify: `client/ui/src/components/ChannelList.svelte:62-71` (kind `builds`, label "Builds", icon), `lib/layout.ts`
  (`LEGACY` unaffected; kind icon map), `lib/quickSwitch.ts` (`PlaceKind` adds `'builds'`), `routes/Shell.svelte:912`
  (route `kind === 'builds'` to `BuildsChannel`), `components/Icon.svelte` (a `hammer` icon if none fits)
- Test: `components/ChannelList.test.ts`, `routes/Shell.test.ts`

- [ ] **Step 1: Failing tests:** `the + channel menu offers Builds and creates one` (api called with `kind: 'builds'`),
  `a builds channel opens the builds view` (Shell renders `BuildsChannel` for it), `the switcher lists builds channels`.
- [ ] **Step 2–4:** FAIL → implement → PASS.
- [ ] **Step 5: Commit** "Builds: create and open a builds channel (#P2)".

### Task 11: Gallery and list

**Files:**
- Create: `components/BuildsHeader.svelte`, `components/BuildCard.svelte`, `components/BuildRow.svelte`,
  `lib/buildPrefs.ts` (+ test)
- Modify: `components/BuildsChannel.svelte`
- Test: `components/BuildsChannel.test.ts`

**Interfaces:**
- Produces: `buildPrefs.ts`: `viewOf(channelId): 'gallery' | 'list'`, `setView(channelId, v)`, `panelOpen(channelId):
  boolean` (default true the first time), `setPanelOpen(channelId, open)` — localStorage key `pulse.builds`, every
  access in try/catch.
- Header: selected game's name or "All games", count, search box (debounced 250 ms → feed with `q`), `Segmented`
  Gallery/List, panel toggle rightmost. "+ Add build" isn't rendered until Task 18 adds it (nothing inert ships).
- Card: thumb (`buildThumb`), "▶ video" tag when it has a link, game caption (small caps), title, two lines of notes,
  "who · when · N comments"; `QUICK DROP` / `ARCHIVED` badges; archived greyed. Row: thumb, caption, title, meta, notes
  snippet. Infinite scroll loads the next page by `before` (page 30).

- [ ] **Step 1: Failing tests:** `shows builds newest first as cards`, `the List toggle switches to rows and is
  remembered per channel`, `a quick drop shows its badge`, `search narrows the list`, `scrolling to the end loads older builds`,
  `an empty channel says how to add one`.
- [ ] **Step 2–4:** FAIL → implement (follow the canvas and `ClipFeed.svelte`'s scroll/paging) → PASS.
- [ ] **Step 5: Commit** "Builds: gallery and list (#P2)".

### Task 12: The right panel

**Files:**
- Create: `components/BuildsPanel.svelte`, `components/BulkArchive.svelte`
- Modify: `components/BuildsChannel.svelte`
- Test: `components/BuildsPanel.test.ts`

Sections: **Games** ("All games" then each game with a count, from `buildFacets`, refetched 1 s after a build event as
`ClipsChannel` does with facets); **Show archived** checkbox (sets the feed's `archived`, counts follow it); **Archive
old builds**: game `Select` (defaults to the selected game, "All games" option), "saved before" (date input, "30 days
ago", "90 days ago"), "Archive matching builds…" → `bulkArchiveBuilds(dry_run: true)` → a confirm dialog listing the
titles and count → `dry_run: false` → toast "Archived N builds". Zero matches: the button says "No builds match".

- [ ] **Step 1: Failing tests:** `clicking a game filters the view`, `counts follow Show archived`, `the bulk archive
  preview lists what it will archive before confirming`, `the confirm sends the same filter without dry run`,
  `a non-owner's preview counts only their own builds` (renders what the server returns), `the panel collapses and stays collapsed`.
- [ ] **Step 2–4:** FAIL → implement → PASS.
- [ ] **Step 5: Commit** "Builds: games, Show archived and bulk archive panel (#P2)".

### Task 13: Older clients are told to update

**Files:** Modify `server/src/gateway/socket.rs:83`. Test: `server/tests/gateway.rs`.

Deploy-gated: merge this only after a client release that speaks protocol 5 is published (the release that carries
Tasks 8–12). Until then no release can show a builds channel, and nobody can create one without that release.

- [ ] **Step 1: Failing test** *(Review Focus 1)*: `a_version_4_hello_is_told_to_update` (Hello with
  `client_version: 4` → close 4005).
- [ ] **Step 2–4:** FAIL → `MIN_CLIENT_VERSION = 5` with a doc line naming builds → PASS.
- [ ] **Step 5: Commit** "Builds: protocol-4 clients update before they meet a builds channel (Closes #P2)".

---

## Phase 3: Build page and adding

### Task 14: Extract the thread chat

**Files:**
- Create: `components/ThreadChat.svelte`
- Modify: `components/RepliesPanel.svelte` (keeps its header and pinned root; the list, pending, older-loading and
  Composer move into ThreadChat)
- Test: `components/RepliesPanel.test.ts` (unchanged, must still pass), `components/ThreadChat.test.ts`

**Interfaces:**
- Produces: `ThreadChat` props `{ root: Message; serverId: string; placeholder: string; authorTag?: string | null }`
  (`authorTag`: the user id whose comments get an "author" tag).

- [ ] **Step 1:** Failing test `ThreadChat tags the root author's replies "author"`; existing RepliesPanel tests green.
- [ ] **Step 2–4:** FAIL → extract → PASS (`pnpm vitest run src/components/RepliesPanel` too).
- [ ] **Step 5: Commit** "Threads: the replies chat as its own component (#P3)".

### Task 15: The build page

**Files:**
- Create: `components/BuildView.svelte` (pure: props only, no store; title caption `BUILDS · <GAME>`, meta line
  "<author> · saved <time> · updated <time>", media tiles, notes markdown), `components/BuildPage.svelte` (BuildView +
  Actions menu + ThreadChat + Back), `components/BuildActions.svelte`
- Modify: `components/BuildsChannel.svelte` (open replaces browse; Back restores filter and scroll),
  `lib/openMessage.ts` (builds and comments on builds open the page, `ui.openBuild`), `lib/store.svelte.ts`
  (`ui.openBuild: { channelId: string; messageId: string } | null`), `components/InboxPopover.svelte:98` and
  `components/SearchResults.svelte:42` (label "build" / "comment on build")
- Test: `components/BuildPage.test.ts`, `lib/openMessage.test.ts`

**Interfaces:**
- Produces: `BuildView` props `{ title, gameName, authorName, savedAt, updatedAt, notes, screenshots: { id, url, w, h }[],
  link: BuildLink | null, archived: boolean, onImage?: (i: number) => void, video?: Snippet }` (Tasks 20 and 25 reuse it).
  `BuildActions` props `{ message: Message; canEdit: boolean; canArchive: boolean; canDelete: boolean; items: ('pin' | 'popout')[] }`
  (phase 3 passes `items: []`; phases 4 and 5 add theirs).

Screenshots: `api.attachmentUrls(messageId)` as `MessageFiles` does; up to 4 tiles then "+N"; a click opens
`ImageViewer` at that index. Actions for the author: Edit, Archive/Restore, Delete; for the owner: Archive/Restore,
Delete; others: none (menu hidden when empty). Delete asks to confirm.

- [ ] **Step 1: Failing tests:** `the page shows caption, meta, notes and four tiles with +N`, `a screenshot opens the
  image viewer`, `the author sees Edit, Archive and Delete; sam sees no Actions; the owner sees Archive and Delete`,
  `the author's comments are tagged author`, `Back returns to the browse view with its filter and scroll`,
  `an inbox comment on a build opens its page`.
- [ ] **Step 2–4:** FAIL → implement → PASS.
- [ ] **Step 5: Commit** "Builds: the build page with comments (#P3)".

### Task 16: The in-app video

**Files:**
- Create: `components/BuildVideo.svelte`
- Modify: `lib/embeds.ts` (`linkAsEmbed(link: BuildLink): LinkEmbed`)
- Test: `components/BuildVideo.test.ts`

A YouTube link (`provider === 'youtube'` with `provider_id`) plays through the same player path as `LinkEmbed.svelte`
(`playerSrc`, `play`/`stop` in `linkPreviews.svelte`, sandbox from `sandboxFor`), with an Enlarge toggle that fills the
page's media area; leaving the page calls `stop`. Anything else is a link card (title, thumb) opening in the browser.

- [ ] **Step 1: Failing tests:** `a youtube build plays in place on click`, `enlarge fills the media area`, `leaving
  the page stops the player`, `a planner link is a card that opens in the browser`.
- [ ] **Step 2–4:** FAIL → implement → PASS.
- [ ] **Step 5: Commit** "Builds: play the build's video inside Pulse (#P3)".

### Task 17: Core: upload screenshots, create and update

**Files:**
- Modify: `client/src-tauri/src/attachments/send.rs` (extract `upload_staged` from `Sender::run`; `run` calls it),
  `client/src-tauri/src/builds.rs`, `client/src-tauri/src/lib.rs`, `client/ui/src/lib/tauri.ts`

**Interfaces:**
- Produces:
  - `pub async fn upload_staged(&self, api: &Api, token: &str, stager: &Stager, channel: ChannelId, nonce: &str, staged: &[String], on_progress: impl Fn(Progress) + Send + Sync + 'static) -> Result<Vec<AttachmentId>, SendError>` on `Sender`
  - commands `create_build(channel_id, body: CreateBuildRequest, staged: Vec<String>) -> Message` (the core uploads
    `staged` in order under `body.nonce`, appends their ids to `body.attachments`, then posts) and
    `update_build(message_id, channel_id, body: UpdateBuildRequest, order: Vec<String>) -> Message` (`order` items are
    `id:<attachment id>` for kept files or `staged:<staged id>` for new ones; the core uploads the staged ones and
    writes `body.attachments` in that order). TS: `createBuild`, `updateBuild`.

- [ ] **Step 1: Failing tests:** `create_build_uploads_then_posts_in_order`, `update_build_keeps_ids_and_fills_placeholders`,
  `a_retried_create_reuses_uploaded_files` (same nonce), and the existing `attachments::send` tests stay green.
- [ ] **Step 2–4:** `cargo test -p pulse-client --lib builds:: attachments::` FAIL → implement → PASS.
- [ ] **Step 5: Commit** "Builds: core uploads screenshots and saves builds (#P3)".

### Task 18: Quick add

**Files:**
- Create: `components/BuildQuickAdd.svelte`, `lib/buildPaste.ts` (+ test)
- Modify: `components/BuildsHeader.svelte` ("+ Add build" opens the quick field), `components/BuildsChannel.svelte`
  (a paste with focus anywhere in the browse view that isn't the search box goes to quick add)
- Test: `components/BuildQuickAdd.test.ts`

**Interfaces:**
- Produces: `classifyPaste(data: DataTransfer): { kind: 'image'; file: File } | { kind: 'link'; url: string } | { kind: 'text'; text: string } | null`
  (an image wins; a single http(s) URL is a link; other text is text, trimmed, first 120 chars).

Enter saves at once (`createBuild`): link → `link_url` only; image → staged screenshot only; text → `title`. The
pending card shows until `MessageCreated` arrives; the title then fills in from the unfurl's `MessageUpdated` and a
guessed game shows "guessed" with a × that clears it (`updateBuild` with `game_id: null`). "Add details" opens the full
form (Task 19) prefilled instead of saving.

- [ ] **Step 1: Failing tests:** `pasting a link and Enter saves a quick drop`, `the title fills in when the unfurl
  lands`, `a guessed game shows guessed and one click clears it`, `pasting an image saves an Untitled build`, `typed
  text becomes the title`, `Add details opens the form instead`, `a paste in the search box stays in the search box`.
- [ ] **Step 2–4:** FAIL → implement → PASS.
- [ ] **Step 5: Commit** "Builds: quick add by paste (#P3)".

### Task 19: The full form and Edit

**Files:**
- Create: `components/BuildForm.svelte`
- Modify: `components/BuildActions.svelte` (Edit opens it), `components/BuildPage.svelte`
- Test: `components/BuildForm.test.ts`

Fields: Title (required, counter at 120), Game (`GamePicker` with a new optional `addLabel` prop, so this form
reads "+ Use 'X' as a new game" (§4.3) while clips keep "Add 'X'"; it calls `api.addGame`),
Build link, Screenshots (drop, paste, pick via `api.pickAttachmentFiles`/`stageAttachmentBytes`; drag to reorder; ×
to remove; at most 10, the 11th says "Up to 10 screenshots."), Notes. A `paste` capture on the form root takes any
image into Screenshots even while focus is in Notes, and leaves text pastes alone. "Mark as sorted" appears on a quick
drop with no game and no notes. Save → `createBuild` or `updateBuild`.

- [ ] **Step 1: Failing tests:** `pasting an image while typing notes adds a screenshot and keeps the notes`,
  `pasting text in notes stays text`, `screenshots reorder and remove`, `an 11th screenshot is refused with a note`,
  `a new game name is added to the registry`, `Save is disabled without a title`, `Edit opens prefilled and saves an
  update`, `Mark as sorted clears the quick drop badge`.
- [ ] **Step 2–4:** FAIL → implement → PASS.
- [ ] **Step 5: Commit** "Builds: the full form, image paste and edit (Closes #P3)".

---

## Phase 4: Quick access

### Task 20: Pin to side

**Files:**
- Create: `lib/pinnedBuild.svelte.ts` (+ test), `components/PinnedBuild.svelte`
- Modify: `routes/Shell.svelte` (`rightKind` gains `'build'`: above members/group, below search and an open voice chat;
  `RIGHT_DEFAULT.build`), `components/BuildActions.svelte` (`'pin'` item: Pin to side / Unpin), `lib/state.ts` (keep
  the pinned build current from events)
- Test: `routes/Shell.test.ts`, `components/PinnedBuild.test.ts`

**Interfaces:**
- Produces: `pinnedBuild` store `{ channelId: string; messageId: string } | null`, `pinBuild(channelId, messageId)`,
  `unpinBuild()`; persisted in localStorage `pulse.pinnedBuild` (try/catch); on start `api.getBuild` refetches it.

`PinnedBuild` renders `BuildView` with a header (title, Open, ×). A 404 on refetch, a `MessageDeleted` for it, or its
channel leaving the store shows "This build is gone." with a Dismiss that unpins.

- [ ] **Step 1: Failing tests:** `pinning shows the build in the right column`, `it stays when moving to a DM and a
  voice channel`, `pinning another replaces it`, `the pin survives a restart` (re-render with the stored key → `getBuild` called),
  `a_pinned_build_that_is_deleted_unpins_with_a_note` *(Review Focus 5)*, `search results still take the column while shown`.
- [ ] **Step 2–4:** FAIL → implement → PASS.
- [ ] **Step 5: Commit** "Builds: pin a build beside any view (#P4)".

### Task 21: Quick peek

**Files:**
- Create: `components/BuildPeek.svelte`
- Modify: `components/BuildCard.svelte`, `components/BuildRow.svelte`
- Test: `components/BuildPeek.test.ts`

Hover ≥ 600 ms (timer as `lib/hoverIntent.ts`), or focus + Space, shows a floating card (title, first 4 lines of notes,
Open, Pin to side) placed with `lib/menuPlace.ts`; leaving both card and peek closes it.

- [ ] **Step 1: Failing tests:** `hovering 600 ms opens the peek, less does not`, `Open goes to the page`, `Pin to side pins it`, `Escape closes it`.
- [ ] **Step 2–4:** FAIL → implement → PASS.
- [ ] **Step 5: Commit** "Builds: quick peek (#P4)".

### Task 22: Ctrl+K `builds:`

**Files:**
- Create: `lib/quickBuilds.ts` (+ test)
- Modify: `lib/quickSwitch.ts` (`Row` gains `{ type: 'build'; message: Message; archived: boolean }`),
  `components/QuickSwitcher.svelte` (a Builds section from a debounced searcher, as Messages; Space on a build row opens
  `BuildPeek`), `routes/Shell.svelte` (Enter opens the page via `openMessage`)
- Test: `components/QuickSwitcher.test.ts`

**Interfaces:**
- Produces: `parseBuildsQuery(raw: string, games: Game[]): { only: boolean; gameKey: string | null; text: string } | null`
  — `builds:` prefix sets `only` (places and actions hidden); the first word(s) after it pick a game when they match
  exactly one distinct `name_key` by name prefix **or by initials** ("tfd" → "The First Descendant"); otherwise all of
  it is text. Plain queries also get the Builds section under the instant ones.

- [ ] **Step 1: Failing tests:** `"raven" lists matching builds`, `"builds:" hides places and actions`, `"builds: tfd"
  filters by The First Descendant`, `an ambiguous prefix is searched as text`, `archived builds come last, marked`,
  `Enter opens the build page`, `Space opens a peek`.
- [ ] **Step 2–4:** FAIL → implement → PASS.
- [ ] **Step 5: Commit** "Builds: find builds from Ctrl+K (Closes #P4)".

---

## Phase 5: The pop-out window

Security model (goes into `SECURITY.md` §4 in Task 25): Tauri 2 lets **every** app-origin window call **every**
registered command unless something stops it, so a capability file alone can't make a second app window read-only.
This phase adds (a) an IPC gate in front of `generate_handler!` that allows the window labelled `build` exactly three
commands, (b) a `build` capability with only event listen/unlisten, (c) navigation locked to the app origin, links to
the browser, and (d) data handed over by the core for one message id only.

### Task 23: The IPC gate and the pop-out capability

**Files:**
- Create: `client/src-tauri/src/ipc_gate.rs`, `client/src-tauri/capabilities/build.json`
- Modify: `client/src-tauri/src/lib.rs:417` (wrap the handler), `client/src-tauri/tests/capabilities.rs`

**Interfaces:**
- Produces: `pub const BUILD_LABEL: &str = "build";` `pub fn allowed(label: &str, command: &str) -> bool` — `"main"`:
  all; `BUILD_LABEL`: only `build_window_snapshot`, `build_window_image_urls`, `set_build_window_on_top`; anything else
  (incl. `embed`): none.
- `lib.rs`: `let handler = tauri::generate_handler![…]; .invoke_handler(move |invoke| { if !ipc_gate::allowed(invoke.message.webview().label(), invoke.message.command()) { invoke.resolver.reject("Not allowed in this window."); return true } handler(invoke) })`.
- `build.json`: `"windows": ["build"]`, permissions exactly `core:event:allow-listen`, `core:event:allow-unlisten`.

- [ ] **Step 1: Failing tests:** unit `main_may_call_anything`, `the_build_window_may_call_only_its_three_commands`,
  `the_embed_window_may_call_nothing`; in `tests/capabilities.rs` replace `only_the_default_capability_exists_and_targets_main`
  with `capabilities_are_default_for_main_and_build_for_the_pop_out` (files `["build.json","default.json"]`, `embed`
  named by neither, no `webviews`/`remote`) and add `the_build_window_may_only_listen`.
- [ ] **Step 2–4:** `cargo test -p pulse-client --lib ipc_gate:: --test capabilities` FAIL → implement → PASS.
- [ ] **Step 5: Commit** "Pop-out: an IPC gate per window label (#P5)".

### Task 24: The pop-out window in the core

**Files:**
- Create: `client/src-tauri/src/build_window.rs`
- Modify: `client/src-tauri/src/lib.rs` (state, commands)

**Interfaces:**
- Produces:
  - `#[derive(Serialize, Deserialize, Clone)] pub struct BuildSnapshot { message_id: MessageId, title, game_name: Option<String>, author_name, saved_at, updated_at: Option<String>, notes: Option<String>, link: Option<BuildLink>, archived: bool }`
  - `open_build_window(app, data, snapshot: BuildSnapshot, on_top: bool)` (main only): replaces an open one; builds
    `WebviewWindowBuilder::new(&app, BUILD_LABEL, WebviewUrl::App("index.html?window=build".into()))`, 420×640 default,
    `always_on_top(on_top)`, same `data_directory` as main, `on_navigation(links::main_navigation)`,
    `on_new_window(links::new_window)`.
  - `update_build_window(app, snapshot)` (main only): stores it, `emit_to(BUILD_LABEL, "build-window://snapshot", …)`.
  - `close_build_window(app, message_id: MessageId)` (main only): closes when it shows that build.
  - `build_window_snapshot(state) -> Option<BuildSnapshot>` (pop-out).
  - `build_window_image_urls(core, state) -> Vec<AttachmentUrl>` (pop-out): `api.attachment_urls` for the stored
    `message_id` only (signed URLs last an hour; the pop-out asks again before they lapse).
  - `set_build_window_on_top(app, on: bool)` (pop-out).

- [ ] **Step 1: Failing tests:** `image_urls_are_only_for_the_snapshots_build` (no snapshot → error; never takes an id
  from the page), `a_new_snapshot_replaces_the_old`, `close_only_closes_the_matching_build`.
- [ ] **Step 2–4:** FAIL → implement → PASS.
- [ ] **Step 5: Commit** "Pop-out: the build window in the core (#P5)".

### Task 25: The pop-out page

**Files:**
- Create: `client/ui/src/BuildPopout.svelte`, `client/ui/src/lib/windowKind.ts` (+ test), `client/ui/src/lib/buildSnapshot.ts` (+ test)
- Modify: `client/ui/src/main.ts` (mount `BuildPopout` when `windowKind(location) === 'build'`, never importing the
  store), `components/BuildActions.svelte` (`'popout'` item), `components/BuildPage.svelte` (sends
  `update_build_window` on the build's events, `close_build_window` on its delete or archive-hidden loss), `SECURITY.md` §4

**Interfaces:**
- Produces: `windowKind(loc: Location): 'main' | 'build'`; `snapshotOf(m: Message, s: AppState): BuildSnapshot`.

`BuildPopout` renders `BuildView` (no comments) from `build_window_snapshot` and `build-window://snapshot` events,
refreshes image URLs every 50 minutes, has an always-on-top toggle (default on), and shows the video as its link card
opening in the browser (inline playback in the pop-out is the parked issue from Task 0).

- [ ] **Step 1: Failing tests:** `windowKind reads ?window=build`, `the pop-out renders the snapshot without the store`,
  `a new snapshot event updates it`, `the on-top toggle calls set_build_window_on_top`, `Pop out sends the snapshot`,
  `deleting the build closes its pop-out` *(Review Focus 5)*.
- [ ] **Step 2–4:** FAIL → implement → PASS; manual check with `just linux-client`: pop out, toggle on top, click a
  notes link (opens in the browser), DevTools `invoke('send_message', …)` from the pop-out is rejected.
- [ ] **Step 5: Commit** "Builds: pop a build out into its own window (Closes #P5)".
