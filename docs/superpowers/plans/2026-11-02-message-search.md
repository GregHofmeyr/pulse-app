# Message search Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** One search box per server (or DM/group) that finds messages by words and combinable filters, with a
results panel that jumps to the message.

**Architecture:** An FTS5 index inside SQLite, kept in sync by triggers (migration 0011, which also fills it from
existing messages). Two read routes (`GET /servers/{id}/search`, `GET /channels/{id}/search`) take structured
parameters, check access the way message listing does, and return hits newest first with highlight ranges. The
Tauri core passes the call through; the Svelte UI parses the box (filters, chips, a Discord-style suggestion
dropdown) and shows a results panel in the right-hand column.

**Tech Stack:** Rust (axum, sqlx/SQLite FTS5, unicode-normalization), ts-rs bindings, Tauri 2 commands, Svelte 5 +
vitest.

**Spec:** `docs/superpowers/specs/2026-11-02-message-search-design.md`, plus the owner's dropdown addition
(relayed 2026-11-02, folded into Task 8 below).

## Global Constraints

- Scope: in a server, every channel of it you can read; in a DM or group, that conversation only; never across servers.
- Never show a message the searcher can't read: another server, a DM they're not in, a deleted message, a group they left.
- No new service; SQLite only. Sending must not slow down noticeably.
- Tokenizer `unicode61 remove_diacritics 2`, prefix indexes 2 and 3. Plain words AND together, prefix-match, case- and accent-insensitive; a quoted phrase must appear as is.
- System lines and deleted messages are never in the index.
- Results newest first, 25 a page; header count shows "50+" past 50.
- Highlights are ranges, never HTML; the client never injects HTML.
- Under 50 ms for a typical query on ~50k messages.
- UI uses `.input`, `.menu-surface` / `.menu-item` (family A), `.btn`, `.pop-in`.
- Public repo: test users alex and sam; no IPs, secrets, emails or real names.

## Decisions (builder's calls)

1. **Contentful FTS5 table keyed through a doc table, not external content.** The indexed body is the message
   content plus quote-line captions (another table), so it isn't one column of `messages`; a contentful table can
   drop a row by rowid without replaying the old text. `messages` has a TEXT primary key, so its implicit rowid can
   change under `VACUUM` (and `VACUUM INTO` backups): the FTS rowid is `search_docs.doc`, an INTEGER PRIMARY KEY that
   survives a vacuum. The index is roughly a second copy of message text: fine at friend-group size.
2. **Index text is the raw content, not a Rust-reduced copy.** Triggers can't call Rust without registering a SQL
   function on every connection (the migration connection, tests' raw connections, the sqlite3 CLI), and a missing
   function would break every write. Mentions are already `@name` in content. Custom emoji `<:name:ID>` index as
   `name` plus the id token; the id can only matter for a query word that is a prefix of a ULID ("01…"), so the
   server re-checks each hit whose content holds a custom emoji token against the displayed text (emoji reduced to
   `:name:`) and drops it if a word doesn't appear there. A dropped hit can make a page shorter than 25; paging
   follows `next_cursor`, never page fullness.
3. **Highlights are computed in Rust over the display text** (`text` in the hit: content with emoji as `:name:`,
   then captions on their own lines), in UTF-16 code units so JS can slice directly. Folding = lowercase + NFD with
   combining marks removed, matching unicode61's behaviour closely enough for highlighting.
4. **Filters are parsed only in the client** (`client/ui/src/lib/searchQuery.ts`, one place, one test file). The
   server tokenizes `q` into words and phrases for the FTS query only.
5. **`has` is one comma-separated parameter** (`has=clip,link`): axum's `Query` can't read repeated keys into a Vec.
6. **`before:` / `after:` are left out** (spec allows it): the dropdown stays short and they weren't asked for.
7. **`has:link`** = content contains `http://`, `https://` or `www.`.
8. **`from:` in a quotes channel** matches the poster, a member speaker (`quote_lines.speaker_id`), or a plain-name
   speaker linked to that member through `speaker_links` at search time.
9. **Count:** `total` (first page only) counts up to 51 matches; the UI shows "50+" at 51.
10. **Rate limit:** reads aren't otherwise limited; search does more work than a list, so it gets its own bucket,
    `search_user: Rate::new(20, 2.0)` (the box debounces, so a person never gets near it).
11. **Access is shared:** `access::can_read(db, user, &Channel)` is the one rule; `channel_for` and the server
    search's channel set both use it.
12. **`/channels/{id}/search` is DMs and groups only** (400 for a server channel: use the server route with `in`).
    An `in` channel outside the server's readable set simply matches nothing.
13. **Shortcut:** Ctrl+F focuses the box (Ctrl+K is free too, but Ctrl+F is what people try first in a chat app);
    the default find-in-page is prevented.
14. **Panel placement:** the results panel replaces the member list / group members in the right column (it also
    shows there in a 1:1 DM, which has none). The quote/clip replies panel lives inside the channel view and may
    sit beside it, so clicking a reply result can open its thread while the results stay open.
15. **Clicking a hit:** a plain message or quote → `jumpTo`. A reply in a quote → `ui.openReplies` (the notification
    click path). A clip, or a reply in a clip → switch to the clips channel and open that clip's replies panel
    (clips channels have no jump list); hits for thread replies carry their `root` message for this.
16. **Name matching for the dropdown:** a new `rankNames` in `lib/mentions.ts`: case-insensitive substring, prefix
    matches first, then by position, then alphabetical. The composer's `suggest` (prefix-only) is left as is.

## Review Focus

- Editing a message into/out of a matching word: the old words must stop matching (Task 2 test `edit_reindexes`).
- A quote edited so a caption changes, and a quote deleted: captions follow, deleted quote vanishes (Task 2).
- FTS syntax in user input (`"`, `*`, `-`, `NEAR`, `OR`, `(`): must never error or change meaning (Task 3 `fts_syntax_is_inert`).
- A server you're not a member of: search behaves as listing does (readable); a DM id on the server route 404s (Task 4).
- Typing in the box while a previous search is in flight: the older answer must not overwrite the newer (Task 7 `stale_answer_ignored`).

---

### Task 1: Protocol types

**Files:** Modify `protocol/src/rest.rs`, `protocol/src/lib.rs`; generated `client/ui/src/lib/protocol/*.ts`.

**Interfaces — Produces:**
- `enum SearchHas { Clip, Quote, Link }` (serde `snake_case`), with `as_str()` / `parse(&str) -> Option<Self>`.
- `struct SearchRequest { q: String, from: Option<UserId>, in_channel: Option<ChannelId>, has: Vec<SearchHas>, mentions: Option<UserId>, cursor: Option<MessageId> }` (`#[serde(default)]` on optionals).
- `struct HighlightRange { start: u32, end: u32 }` (UTF-16 offsets into `text`).
- `struct SearchHit { message: Message, text: String, highlights: Vec<HighlightRange>, root: Option<Message> }`.
- `struct SearchResults { hits: Vec<SearchHit>, next_cursor: Option<MessageId>, total: Option<u32> }`.

- [ ] Test `search_has_round_trips` in rest.rs tests: each variant serializes to `"clip"`/`"quote"`/`"link"` and `parse` inverts `as_str`; `parse("emoji") == None`. Run, fail, implement, pass.
- [ ] Register exports; `cargo test -p pulse-protocol export_bindings`; commit types + bindings.

### Task 2: Migration 0011 and index triggers

**Files:** Create `server/migrations/0011_search.sql`; modify migration-count tests (`tests/clips.rs` `migrated: 10`, `tests/quotes.rs` `assert_eq!(v, 10, …)`); create `server/tests/search_index.rs`.

**Interfaces — Produces:** tables `search_docs(doc INTEGER PRIMARY KEY, message_id TEXT NOT NULL UNIQUE)` and FTS5 `message_search(body)`; a message is indexed iff `kind = 'normal' AND deleted_at IS NULL`; body = `content || (char(10) || group_concat(caption, char(10)))` of its quote lines with a caption.

Triggers: `AFTER INSERT ON messages` (indexed ones), `AFTER UPDATE OF id, content, kind, deleted_at ON messages`
(drop the old doc and FTS row, re-add if still indexed), `AFTER DELETE ON messages`, and `AFTER INSERT/UPDATE/DELETE
ON quote_lines` (re-write the message's FTS row if it has a doc). Backfill in one pass, then FTS `optimize`.

- [ ] Tests in `search_index.rs` (a helper `hits(pool, match) -> Vec<String>` of message ids via the doc join): `insert_is_indexed`, `edit_reindexes` (old word gone, new word found), `soft_delete_unindexes`, `hard_delete_unindexes`, `system_lines_excluded`, `quote_captions_indexed_and_follow_edits`, `deleted_quote_unindexed`, `thread_replies_indexed`, `accent_and_prefix_match` ("cafe" finds "Café", "clip" finds "clips"), `migration_backfills_existing` (DB migrated to 10 with a live message, a soft-deleted one with leftover content, a system line and a quote with a caption, then migrate to 11: only the live message and the quote are found, caption included). Bump `migrated: 11` and `v, 11` expectations. Run, fail, write migration, pass, commit.

### Task 3: Query words, FTS expression, display text and highlights (pure Rust)

**Files:** Create `server/src/search/mod.rs` (`pub mod text; pub mod routes;`), `server/src/search/text.rs`; add `unicode-normalization = "0.1"` to server deps.

**Interfaces — Produces (in `search::text`):**
- `enum Term { Word(String), Phrase(Vec<String>) }`; `fn terms(q: &str) -> Vec<Term>` — quoted runs are phrases, everything else words; each word is split into tokens on non-alphanumerics and folded (each token its own `Word`). Max 16 terms (extra ignored).
- `fn fold(s: &str) -> String` (lowercase, NFD, drop combining marks).
- `fn fts_query(terms: &[Term]) -> Option<String>` — `"tok"*` per word, `"a b"` per phrase, joined by spaces; `None` when empty.
- `fn display_text(content: &str, captions: &[String]) -> String` — custom emoji `<:n:id>`/`<a:n:id>` → `:n:`, captions appended on new lines.
- `fn highlights(text: &str, terms: &[Term]) -> Option<Vec<HighlightRange>>` — merged, sorted UTF-16 ranges; `None` if some term matched nothing.
- `fn has_custom_emoji(content: &str) -> bool` (reuse `messages::reactions::custom_token`, made `pub(crate)`).

- [ ] Unit tests: `terms_split_words_and_phrases` (`hi "big clip" Café-run` → Word hi, Phrase [big, clip], Word cafe, Word run), `fts_syntax_is_inert` (`a" OR -b* NEAR(` builds only quoted tokens), `empty_query_has_no_fts`, `display_reduces_custom_emoji`, `highlights_prefix_and_accents` (`"Clips at the café"` with `clip cafe` → ranges [0,5] and [12,16]), `highlights_phrase_exact`, `highlights_utf16_offsets` (an emoji before the word shifts by 2), `emoji_id_only_match_is_none`. Run, fail, implement, pass, commit.

### Task 4: Search routes, access and rate limit

**Files:** Create `server/src/search/routes.rs`; modify `server/src/access.rs` (add `can_read`, `readable_server_channels`), `server/src/lib.rs` (mod + router), `server/src/limits.rs` (`search_user`); create `server/tests/search.rs`.

**Interfaces:**
- Consumes Task 1 types, Task 3 functions, `messages::routes::{fill, COLS, Row}`.
- Produces `GET /servers/{id}/search` and `GET /channels/{id}/search`, query params `q, from, in, has, mentions, cursor`; 25 per page; `next_cursor` = last row read when 25 were read; `total` on the first page (≤ 51).
- `pub async fn can_read(db, user, ch: &Channel) -> AppResult<bool>`; `pub async fn readable_server_channels(db, user, server) -> AppResult<Vec<Channel>>` (chat kinds only; 404 if no such server).

- [ ] Tests in `tests/search.rs` (alex and sam, server "Main"): `finds_words_newest_first`, `filter_only_from`, `from_matches_quote_speakers_and_links`, `in_one_channel`, `has_clip_quote_link` (clips via the `clips` table rows inserted directly, as tests/clips.rs fixtures do, or the clip route where the harness allows), `mentions_filter_includes_reply_pings`, `combined_filters`, `paging_is_stable` (60 matches, three pages, no dupes, a message sent mid-paging doesn't shift page 2), `total_caps_at_51`, `deleted_never_shows`, `dm_not_member_404`, `left_group_404`, `other_server_never_in_results`, `server_channel_on_channel_route_400`, `thread_reply_carries_root`, `custom_emoji_id_doesnt_match`, `empty_search_400`, `rate_limited_429`. Run, fail, implement, pass, commit.

### Task 5: Performance check

**Files:** `server/tests/search_perf.rs` (`#[ignore]`).

- [ ] Insert 50k synthetic messages over 5 channels directly with SQL (a vocabulary of ~2k words, some rare), then time 5 typical queries through the HTTP route (a common word, a rare word, two words, `from:` only, word + `from:`); print timings; assert each median < 50 ms. Run with `--ignored --nocapture`, record numbers in the commit message.

### Task 6: Tauri core command

**Files:** `client/src-tauri/src/api.rs` (`search_server`, `search_channel`), `commands.rs` (`search_messages`), `lib.rs` (register).

**Interfaces — Produces:** command `search_messages(server_id: Option<ServerId>, channel_id: Option<ChannelId>, query: SearchRequest) -> Result<SearchResults, ApiError>` (server if given, else channel).

- [ ] api.rs live test `search_finds_and_filters` alongside the existing ones: send two messages, search a word and `from`. Run, fail, implement, pass, commit.

### Task 7: UI query parsing, ranking and search store

**Files:** Create `client/ui/src/lib/searchQuery.ts` (+test), `client/ui/src/lib/search.svelte.ts` (+test); modify `lib/mentions.ts` (+test), `lib/tauri.ts`.

**Interfaces — Produces:**
- `type Chip = { kind: 'from' | 'mentions'; id: string; label: string } | { kind: 'in'; id: string; label: string } | { kind: 'has'; value: 'clip' | 'quote' | 'link' }`.
- `FILTERS`: rows `{ key: 'from' | 'in' | 'has' | 'mentions'; syntax; hint }` — "from: user" / "From a specific user", "in: channel" / "In a channel", "has: link, clip or quote" / "Includes a link, clip or quote", "mentions: user" / "Mentions a user".
- `activeTerm(text, caret) -> { start, end, filter: Key | null, value: string }`; `parseText(text) -> { words: string; filters: { key, value }[] }` (unknown `foo:bar` stays a word; quotes kept intact).
- `toRequest(words, chips) -> SearchRequest`.
- `rankNames<T extends {name: string}>(items, q, max = 8)` in mentions.ts.
- Store `search`: `{ open, scope, chips, words, status: 'idle'|'loading'|'done'|'error', hits, total, nextCursor, error }`; `runSearch()`, `loadMore()`, `closeSearch()`, `openHit(hit)`; `api.searchMessages`.

- [ ] Tests: parsing (words, quotes, filters, unknown filter as words), `activeTerm` at start/in a value, `rankNames` (prefix first, then substring, case-insensitive), store `stale_answer_ignored`, `load_more_appends`, `error_then_retry`, `openHit` routes (plain → jumpTo, quote reply → ui.openReplies, clip → ui.openClip). Run, fail, implement, pass, commit.

### Task 8: Search box with chips and suggestion dropdown

**Files:** Create `client/ui/src/components/SearchBox.svelte` (+test); modify `TextChannel.svelte` and `ClipsHeader.svelte` (place it), `ClipsChannel.svelte` (handle `ui.openClip`).

Combobox: `role="combobox"`, `aria-expanded`, `aria-controls`, `aria-activedescendant`; listbox `.menu-surface.pop-in` with `role="option"` rows `.menu-item`, `.hi` on the active one. Empty box or start of a new term → filter rows (no `in:` outside a server); typing `from:`/`mentions:` → ranked members with avatar + `Name`; `in:` → channels; `has:` → the fixed list. Click/Enter/Tab picks; arrows move; Esc closes the list, then (second Esc) clears/blurs. Picking a value makes a chip and returns to the filter rows. Backspace on empty input removes the last chip; chips have ✕. Enter with no list open runs the search. Placeholder "Search Main" / "Search this conversation". Ctrl+F focuses it.

- [ ] Tests: filter rows on focus; no `in:` in a DM; arrows + Enter insert `from:` and keep focus; typing `from:am` lists sam (substring) with prefix matches first; picking makes a chip and filter rows return; backspace removes a chip; Enter runs the search with structured params; ARIA attributes present. Run, fail, implement, pass, commit.

### Task 9: Results panel

**Files:** Create `client/ui/src/components/SearchResults.svelte` (+test), `client/ui/src/lib/highlight.ts` (+test); modify `routes/Shell.svelte` (right column).

- [ ] `highlight.ts`: `segments(text, ranges) -> { text: string; hit: boolean }[]` (clamped, sorted, overlap-safe); test with `<img onerror>` text proving it renders as text.
- [ ] Panel tests: loading, results with count "50+", "reply in quote" / "reply in clip" labels, channel name in server search, empty state "No messages match" listing the filters, error state with Retry, Load more, click calls the hit router, Esc closes. Run, fail, implement, pass, commit.

### Task 10: Checks, review, fixes

- [ ] fmt, clippy (Linux + xwin), server and client tests, cargo deny, types-check, pnpm check/test.
- [ ] One fresh reviewer on the branch diff; fix Critical/Important/Minor test-first.
