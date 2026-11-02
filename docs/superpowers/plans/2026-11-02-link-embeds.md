# Link embeds Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Preview cards under messages that hold links, built once by the server, with click-to-play players
for YouTube, TikTok, Spotify, SoundCloud and Twitch clips through a public embed page on our server, and an
Instagram card that plays in a capability-free pop-out window.

**Architecture:** After a message is sent or edited, the server extracts its first 3 links, unfurls each one
through an SSRF-guarded fetcher (official oEmbed for known providers, OpenGraph otherwise), stores the result in
`link_embeds` (thumbnails as `link_thumb` media), and publishes `MessageUpdated`. Messages carry `embeds` in
their payload. `GET /embed/<provider>/<id>` is a tiny static page that iframes the provider's official player;
the app iframes that page only when it is connected to `https://pulsechat.co.za` (the CSP's one `frame-src`).

**Tech Stack:** Rust (axum 0.8, sqlx/SQLite, reqwest 0.12 with a custom `dns::Resolve`, `image`/libwebp via
`media::process`), ts-rs bindings, Tauri 2.12 (`WebviewWindowBuilder`), Svelte 5 + vitest/jsdom.

**Spec:** `docs/superpowers/specs/2026-11-02-link-embeds-design.md` (with
`docs/superpowers/research/link-embeds.md`). Scope: phases 1 and 2, plus phase 3's card and pop-out.

## Global Constraints

- Build throttle, every cargo command: `CARGO_BUILD_JOBS=4 flock ~/Projects/pulse-app/target/.cargo-serial nice -n 19 ionice -c3 cargo …`; tests add `-- --test-threads=4`. One cargo command at a time. pnpm: `nice -n 19`, node from `~/.config/nvm/versions/node/v22.22.3/bin`.
- Public repo: test users alex/sam/kim/jo; test URLs use `example.com`/`*.test` hosts or fictional ids; no IPs other than documentation/private test ranges, no secrets.
- New protocol fields are `#[serde(default)]`; old clients and servers interoperate.
- Never render provider HTML/markdown/`{@html}`: cards come from structured data only. Iframe URLs come from the embed page only, built from templates plus a validated id.
- Fetcher: http/https only, ports 80 and 443; own DNS resolution, reject any non-public address, connect to the validated IP; redirects by hand, at most 5, each re-checked; 5 s total; HTML at most 1 MB and `text/html` only; images at most 8 MB; user agent `PulseBot/1.0 (+https://pulsechat.co.za)`.
- At most 3 distinct links per message; links in code (fenced or inline) and `<https://…>` are skipped.
- Cache keyed by normalised URL, valid a day (failures: an hour). Concurrency limited; per-user rate limit.
- Click-to-play only, never autoplay before a click; one player at a time.
- Quotes and clips get no previews; no backfill; soft delete removes the message's `link_embeds` rows; hidden is per URL and survives edits; no un-hide.
- `PULSE_PUBLIC_URL` unset = the embed page answers 404. App CSP gains exactly `frame-src https://pulsechat.co.za`.
- Embed page headers: `Referrer-Policy: strict-origin-when-cross-origin`; CSP `default-src 'none'; style-src 'unsafe-inline'; frame-src <provider origin>; frame-ancestors http://tauri.localhost https://tauri.localhost tauri:`.
- Migration number 0012. Media kind `link_thumb`, on disk, kept by the hourly cleanup while a `link_embeds` row references it.

## Review Focus

1. A link the sender edits away while its fetch is in flight must not come back: the unfurl task writes only if the message's content is still what it read (test in Task 5: `an_edit_during_the_fetch_drops_the_stale_card`).
2. A thumbnail hash taken from the cache after the cleanup removed it must not be stored (dead image): the store touches the media row in the write transaction and drops the thumb if it is gone (Task 4: `a_cached_thumb_that_was_cleaned_up_is_not_reused`).
3. URLs with trailing punctuation or inside markdown links/parentheses, e.g. `(see https://example.com/a_(b))`. and `[x](https://example.com/y)`, must give the right URL (Task 2 tests).
4. A message whose author is over the unfurl rate limit still posts; it just gets no cards (Task 5: `over_the_unfurl_limit_the_message_posts_without_cards`).
5. An older client must still parse a `Message` with `embeds` and an unknown provider; a new client must parse a pre-embeds `Message` (Task 1: protocol tests, `EmbedProvider::Unknown` via `#[serde(other)]`).

---

## File Structure

Server (`server/src/embeds/`, new module, like `search/`):
- `mod.rs` – `Unfurler` (config, cache, semaphore, http client), `attach`, `on_send`/`on_edit` spawning, store/publish.
- `links.rs` – `extract(content) -> Vec<String>` (pure).
- `providers.rs` – `Detected`, `detect(&Url)`, id validators, player templates (pure; shared with the embed page).
- `fetch.rs` – `Fetcher`: guarded resolver, manual redirects, caps.
- `meta.rs` – OpenGraph/`<title>` scanner and entity decoding (pure), oEmbed JSON shape.
- `unfurl.rs` – per-provider metadata flow -> `Unfurled`.
- `page.rs` – `GET /embed/{provider}/{id}`.
- `routes.rs` – `POST /messages/{id}/embeds/hide`.
- `server/migrations/0012_link_embeds.sql`.
- Modified: `media/mod.rs` (kind + cleanup), `media/process.rs` (`link_thumb`), `messages/routes.rs` (attach, send/edit/remove hooks), `config.rs` (`EmbedConfig`), `state.rs`, `lib.rs`, `testing/mod.rs`, `limits.rs` (`unfurl_user`).

Protocol: `protocol/src/rest.rs` (`LinkEmbed`, `EmbedProvider`, `EmbedKind`, `EmbedThumb`, `HideEmbedRequest`, `Message.embeds`), `protocol/src/lib.rs` exports.

Client core: `client/src-tauri/src/api.rs` (`hide_embed`), `commands.rs` (`hide_embed`, `embed_origin`), new `embed_window.rs` (pop-out), `lib.rs` registration, `tauri*.conf.json` CSP, `tests/csp.rs`.

UI: `lib/embeds.ts` (pure helpers), `lib/linkPreviews.svelte.ts` (setting + the one active player), `components/LinkEmbed.svelte`, `MessageItem.svelte`, `ChatSettings.svelte`, `lib/tauri.ts`.

Deploy/docs: `deploy/compose.yml` (`PULSE_PUBLIC_URL: https://${PULSE_DOMAIN:?}`), `deploy/deploy-selftest.sh`, `SECURITY.md`.

---

### Task 1: Protocol, migration, storage and attach

**Files:**
- Modify: `protocol/src/rest.rs`, `protocol/src/lib.rs`, `protocol/src/gateway.rs` (tests module)
- Create: `server/migrations/0012_link_embeds.sql`, `server/src/embeds/mod.rs`
- Modify: `server/src/lib.rs`, `server/src/media/mod.rs`, `server/src/messages/routes.rs`, every `Message { … }` literal (`server/src/clips/routes.rs`, `server/tests/gateway.rs`, `server/tests/review_fixes.rs`)
- Test: `server/tests/embeds_store.rs`, protocol unit tests

**Interfaces:**
- Produces (protocol):
  ```rust
  #[serde(rename_all = "snake_case")] pub enum EmbedProvider { Youtube, Tiktok, Spotify, Soundcloud, Twitch, Instagram, Generic, #[serde(other)] Unknown }
  #[serde(rename_all = "snake_case")] pub enum EmbedKind { Card, Player }
  pub struct EmbedThumb { pub hash: String, pub width: u32, pub height: u32 }
  pub struct LinkEmbed { pub url: String, pub provider: EmbedProvider, pub kind: EmbedKind,
      pub provider_id: Option<String>, pub title: Option<String>, pub description: Option<String>,
      pub site_name: Option<String>, pub author: Option<String>, pub thumb: Option<EmbedThumb>,
      /// YouTube start, seconds.
      pub start: Option<u32>, /// 9:16 player (TikTok, Shorts, reels).
      pub vertical: bool }
  pub struct HideEmbedRequest { pub url: Option<String> }   // None = all of the message's previews
  Message { …, #[serde(default)] pub embeds: Vec<LinkEmbed> }
  ```
  All derive `Clone, Debug, PartialEq, Serialize, Deserialize, TS`; every `LinkEmbed` field after `kind` is `#[serde(default)]`.
- Produces (server): table `link_embeds(message_id TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE, position INTEGER NOT NULL, url TEXT NOT NULL, provider TEXT NOT NULL, provider_id TEXT, kind TEXT NOT NULL, title TEXT, description TEXT, site_name TEXT, author TEXT, thumb_hash TEXT, thumb_width INTEGER, thumb_height INTEGER, start_secs INTEGER, vertical INTEGER NOT NULL DEFAULT 0, hidden INTEGER NOT NULL DEFAULT 0, fetched_at TEXT NOT NULL, PRIMARY KEY (message_id, url)) STRICT` plus index on `thumb_hash`.
  `MediaKind::LinkThumb` (`"link_thumb"`, disk).
  `pub async fn embeds::attach(db: &SqlitePool, msgs: &mut [Message]) -> AppResult<()>` – non-hidden rows of live messages, ordered by `position`; called from `messages::routes::fill` and `load`.
  `pub async fn embeds::on_delete(tx: &mut SqliteConnection, id: MessageId) -> AppResult<()>` – deletes the message's rows; called in `remove`'s transaction.

- [ ] **Step 1: Failing tests.** Protocol: a `Message` JSON without `embeds` parses with `embeds == []`; a `LinkEmbed` with `"provider":"vimeo"` parses as `EmbedProvider::Unknown`; a `LinkEmbed` with only `url`, `provider`, `kind` parses. Server `tests/embeds_store.rs`: `messages_carry_their_embed_rows` (insert a row by SQL for a sent message; `GET /channels/{id}/messages` shows it, hidden rows and other messages' rows don't); `deleting_a_message_removes_its_embeds` (soft delete → zero rows); `cleanup_keeps_a_referenced_link_thumb` and `cleanup_drops_a_link_thumb_once_its_message_is_deleted` (a `link_thumb` media row older than the grace, via `media::cleanup(db, dir, Duration::ZERO)`).
- [ ] **Step 2:** Run `cargo test -p pulse-protocol` and `cargo test -p pulse-server --test embeds_store` → fail (compile errors).
- [ ] **Step 3:** Implement: types, export in `export_all` (`LinkEmbed`, `EmbedProvider`, `EmbedKind`, `EmbedThumb`, `HideEmbedRequest`), migration, `embeds::attach`/`on_delete`, cleanup clause `AND hash NOT IN (SELECT thumb_hash FROM link_embeds WHERE thumb_hash IS NOT NULL)`, `embeds: vec![]` in literals.
- [ ] **Step 4:** Tests pass; `cargo test -p pulse-protocol export_bindings` regenerates TS; update the TS `Message` literals in `client/ui/src` (`clip: null` → add `embeds: []`) so `pnpm check` stays clean.
- [ ] **Step 5:** Commit `feat(embeds): link_embeds storage, protocol fields and cleanup`.

### Task 2: Link extraction and provider parsing (pure)

**Files:** Create `server/src/embeds/links.rs`, `server/src/embeds/providers.rs` (unit tests inside).

**Interfaces:**
- `pub fn links::extract(content: &str) -> Vec<String>` – first 3 distinct (by `normalise`) http(s) URLs; skips fenced code, inline code and `<https://…>`; trims trailing `.,;:!?'"` and an unbalanced `)`/`]`; a URL inside `[text](url)` is found.
- `pub fn links::normalise(url: &str) -> Option<String>` – `reqwest::Url` parse, http/https only, fragment dropped.
- `pub struct Detected { pub provider: EmbedProvider, pub id: String, pub start: Option<u32>, pub vertical: bool }`
- `pub enum Detection { Known(Detected), Short /* needs a redirect resolve: vm/vt.tiktok.com, tiktok.com/t/, spotify.link, on.soundcloud.com */, Unknown }`
- `pub fn providers::detect(url: &Url) -> Detection` (host-based; scheme and port ignored).
- `pub fn providers::valid_id(p: EmbedProvider, id: &str) -> bool` and `pub fn providers::player_url(p: EmbedProvider, id: &str, start: Option<u32>, parent_host: &str) -> Option<String>` – the only place iframe URLs are built.
- Id formats: YouTube `[A-Za-z0-9_-]{11}`; TikTok `\d{5,25}`; Spotify `<type>:<id>` with type in track/album/playlist/episode/show/artist and id `[A-Za-z0-9]{22}`; SoundCloud `user/track` or `user/sets/name`, segments `[a-z0-9_-]{1,100}`; Twitch clip slug `[A-Za-z0-9_-]{1,100}`; Instagram `p:<code>` or `reel:<code>`, code `[A-Za-z0-9_-]{5,64}`.
- Player templates: YouTube `https://www.youtube-nocookie.com/embed/<id>?autoplay=1[&start=N]`; TikTok `https://www.tiktok.com/player/v1/<id>?autoplay=1&rel=0`; Spotify `https://open.spotify.com/embed/<type>/<id>`; SoundCloud `https://w.soundcloud.com/player/?url=https%3A%2F%2Fsoundcloud.com%2F<path>&auto_play=true`; Twitch `https://clips.twitch.tv/embed?clip=<slug>&parent=<parent_host>&parent=tauri.localhost&autoplay=true`; Instagram `https://www.instagram.com/<p|reel>/<code>/embed/`.

- [ ] **Step 1: Failing tests:** `extract` – code blocks, inline code, `<…>` suppression, cap of 3, distinctness, trailing punctuation, parentheses, markdown links, non-http schemes ignored. `detect` – every URL form: `youtube.com/watch?v=`, `m.`/`music.` hosts, `youtu.be/ID?t=1m30s` (start 90), `&t=90s`, `start=`, `/shorts/ID` (vertical), `/embed/`, `/live/`; TikTok `@alex/video/ID`, `m.tiktok.com/v/ID.html`, `/embed/v2/ID`, `/player/v1/ID`, short forms → `Short`; Spotify incl. `/intl-de/track/…`; SoundCloud track and set, reserved first segments (`discover`, `search`, `you`, `stream`, `upload`, `pages`, `charts`) → Unknown; Twitch `clips.twitch.tv/SLUG`, `twitch.tv/chan/clip/SLUG`; Instagram `/p/`, `/reel/`, `/reels/`, `/tv/`, `/<user>/reel/<code>/`; lookalike hosts (`youtube.com.example.com`, `notyoutube.com`) → Unknown. `player_url` refuses invalid ids and escapes nothing it doesn't need (ids are validated).
- [ ] **Step 2:** Run `cargo test -p pulse-server --lib embeds::` → fail.
- [ ] **Step 3:** Implement (no regex crate; hand-written char checks).
- [ ] **Step 4:** Pass. **Step 5:** Commit `feat(embeds): link extraction and provider id parsing`.

### Task 3: The guarded fetcher (SSRF)

**Files:** Create `server/src/embeds/fetch.rs`; modify `server/src/config.rs` (`EmbedConfig`), `server/src/testing/mod.rs`. Test: `server/tests/embeds_fetch.rs`.

**Interfaces:**
- `config::EmbedConfig { pub public_url: Option<reqwest::Url>, pub endpoints: Endpoints, pub timeout: Duration /*5 s*/, pub allowed_ips: Vec<IpAddr> /*tests only; empty from env*/, pub extra_ports: Vec<u16> /*tests only*/, pub hosts: Vec<(String, IpAddr)> /*tests only: fixed DNS answers*/ }`; `Endpoints { youtube, tiktok, spotify, soundcloud, instagram: String }` oEmbed bases defaulting to `https://www.youtube.com/oembed`, `https://www.tiktok.com/oembed`, `https://open.spotify.com/oembed`, `https://soundcloud.com/oembed`, `https://graph.facebook.com/v25.0/instagram_oembed`. `Config.embeds: EmbedConfig`; `from_env` reads `PULSE_PUBLIC_URL` (empty = None; must be http(s) with a host and no path/query, else an error naming the variable).
- `pub fn fetch::is_public(ip: IpAddr) -> bool` – false for 0/8, 10/8, 100.64/10, 127/8, 169.254/16, 172.16/12, 192.0.0/24, 192.168/16, 198.18/15, 224/4, 240/4, broadcast, `::`, `::1`, `fc00::/7`, `fe80::/10`, `ff00::/8`, IPv4-mapped/compatible of any of those, 64:ff9b::/96 of them.
- `pub struct Fetcher` with `Fetcher::new(cfg: &EmbedConfig) -> Fetcher`; `pub async fn get(&self, url: &str, want: Want) -> Result<Fetched, FetchError>` where `Want::{Html, Json, Image}` sets accepted content types and caps (HTML 1 MB, JSON 256 KB, image 8 MB); `Fetched { pub final_url: Url, pub content_type: String, pub body: Vec<u8> }`. `pub async fn resolve_redirects(&self, url: &str) -> Result<Url, FetchError>` – follows up to 5 redirects and returns the first URL `providers::detect` knows (used for short links), without fetching it.
- reqwest client: `redirect::Policy::none()`, `.no_proxy()`, `.dns_resolver(Arc<GuardResolver>)` (rejects the name if any answer fails `is_public` and isn't in `allowed_ips`), user agent from Global Constraints. IP-literal hosts are checked before the request (reqwest skips the resolver for them). Ports: 80/443 + `extra_ports`. Userinfo in the URL refused. Whole call (incl. redirects) under `timeout`. Body read in chunks, stops at the cap (HTML: over the cap is truncated, not an error; JSON/image: over the cap is an error).

- [ ] **Step 1: Failing tests** (fake axum servers on 127.0.0.1:0; `hosts` maps `*.test` names to 127.0.0.1): `refuses_private_loopback_link_local_and_metadata_addresses` (127.0.0.1, `[::1]`, 10.0.0.1, 169.254.169.254, 100.64.0.1, `[::ffff:127.0.0.1]`, a name resolving to 127.0.0.1 with `allowed_ips` empty) – no connection reaches the fake server (hit counter stays 0); `refuses_a_name_with_any_private_answer` (unit test on the resolver with [public, private] answers – the rebinding case); `refuses_other_ports_and_schemes` (`:8080`, `ftp:`, `file:`, userinfo); `rechecks_every_redirect` (allowed fake → 302 to a name mapped to 10.0.0.1 → refused; to `http://127.0.0.1:<p>/` with 127.0.0.1 not allowed → refused); `stops_after_five_redirects`; `times_out` (handler sleeps past a 300 ms timeout); `caps_html_and_refuses_oversized_images`; `html_only_for_page_fetches` (image/png body for `Want::Html` → error); `sends_the_honest_user_agent`. Unit test: `is_public` table.
- [ ] **Step 2:** Run → fail. **Step 3:** Implement. **Step 4:** Pass.
- [ ] **Step 5:** Commit `feat(embeds): SSRF-guarded fetcher`.

### Task 4: Metadata, thumbnails and the cache

**Files:** Create `server/src/embeds/meta.rs`, `server/src/embeds/unfurl.rs`; modify `server/src/media/process.rs` (`link_thumb`), `server/src/embeds/mod.rs` (`Unfurler`), `state.rs`, `lib.rs`, `testing/mod.rs`. Test: `server/tests/embeds_unfurl.rs` (+ unit tests in `meta.rs`, `process.rs`).

**Interfaces:**
- `pub fn meta::page_meta(html: &str) -> PageMeta { title, description, site_name, image }` – `og:*` first, then `twitter:*`, then `<title>`/`description`; stops at `</head>`; decodes `&amp; &lt; &gt; &quot; &#39; &apos; &nbsp; &#N; &#xH;`; never interprets markup.
- `pub fn meta::clean(s: &str, max_chars: usize) -> Option<String>` – drops control chars, collapses whitespace, truncates with `…`; caps: title 200, description 300, site name 80, author 80.
- `pub fn process::link_thumb(input: &[u8]) -> Result<Processed, ProcessError>` – PNG/JPEG/GIF/WebP, first frame only, fitted within 640×640 (never enlarged), still WebP, 8 MB input.
- `pub struct Unfurled { provider, kind, provider_id, title, description, site_name, author, thumb: Option<EmbedThumb>, start, vertical }` (= `LinkEmbed` minus `url`).
- `pub struct Unfurler` (in `AppState.embeds: Arc<Unfurler>`): `Unfurler::new(cfg: &EmbedConfig) -> Self`; `pub async fn unfurl(&self, db: &SqlitePool, media_dir: &Path, by: UserId, url: &str) -> Option<Unfurled>` – cache (normalised URL → `Unfurled` or failure, 24 h / 1 h, at most 1000 entries, oldest evicted), a global semaphore of 4, then `unfurl::run`.
- Per-provider flow (`unfurl.rs`): `Short` → `resolve_redirects` then detect; YouTube/TikTok/Spotify/SoundCloud → their oEmbed (`?url=<canonical URL>&format=json`), title/author/thumbnail_url/provider_name; Twitch → OG of the clip page; Instagram → oEmbed (title used as caption if present), and a card even when it fails (`site_name` "Instagram", no thumb; the UI draws the placeholder); Generic → OG of the page; nothing usable (no title and no image) → `None`. Player providers whose metadata fetch fails → `None` (spec: a failed fetch means no card), except Instagram.
- Thumbnails: `Fetcher::get(thumbnail_url, Want::Image)` → `run_processing(link_thumb)` → `media::put(…, MediaKind::LinkThumb, …, by)`. A failed thumbnail leaves the card without one.
- `pub async fn embeds::store(db, message_id, content_when_read: &str, rows: Vec<(usize, String, Unfurled)>) -> AppResult<bool>` – one `begin_write` transaction: returns false (writes nothing) unless the message is live with the same content; for each thumb, `UPDATE media SET created_at = now WHERE hash = ?` and drops the thumb if no row changed; `INSERT … ON CONFLICT(message_id, url) DO UPDATE` keeping `hidden`.

- [ ] **Step 1: Failing tests.** `meta.rs` unit: OG precedence, entity decoding, a `<script>`/attribute-injection title comes back as plain text, relative `og:image` resolved against the final URL (in `unfurl`), stops at `</head>`. `process.rs` unit: `link_thumb` fits a 2000×1000 PNG to 640×320, keeps a 100×50 one, takes an animated GIF's first frame only. `tests/embeds_unfurl.rs` (fake provider server for all oEmbed endpoints + pages + a PNG): `youtube_card_from_oembed_with_a_webp_thumbnail` (stored media kind `link_thumb`, file exists); `tiktok_short_link_resolves_to_a_vertical_player`; `spotify_soundcloud_and_twitch_cards`; `instagram_card_without_oembed_is_a_placeholder_card`; `generic_page_gets_an_og_card`; `a_page_without_metadata_gets_nothing`; `the_cache_serves_a_second_unfurl_without_fetching` (hit counter); `a_cached_thumb_that_was_cleaned_up_is_not_reused` (delete the media row, then `store` → embed without thumb); `store_writes_nothing_after_the_content_changed`.
- [ ] **Step 2:** Run → fail. **Step 3:** Implement. **Step 4:** Pass.
- [ ] **Step 5:** Commit `feat(embeds): oEmbed/OpenGraph unfurling, link thumbnails and cache`.

### Task 5: Hooking messages: send, edit, hide

**Files:** Modify `server/src/messages/routes.rs`, `server/src/limits.rs`, `server/src/embeds/mod.rs`; create `server/src/embeds/routes.rs`. Test: `server/tests/embeds.rs`.

**Interfaces:**
- `LimitsConfig.unfurl_user: Rate` = `Rate::per_minute(20)` (unlimited in tests' `unlimited()`); over it, the unfurl is skipped.
- `pub fn embeds::spawn_unfurl(s: &AppState, message_id: MessageId, author: UserId, content: String)` – spawned after `send` commits (normal messages only: not quotes or clips) and after `edit` commits; extracts links, skips URLs that already have a row, unfurls the rest concurrently, `store`s, then `load`s and publishes `Event::MessageUpdated`.
- `edit` – in its UPDATE, also deletes non-hidden rows whose URL is no longer among `links::extract(new content)`; the published edit carries the remaining embeds (`attach` after the update).
- `POST /messages/{id}/embeds/hide` with `HideEmbedRequest` – author only (`own_message`), `mutate_user` limited; sets `hidden = 1` for that URL (or all); 404 for a URL the message has no row for; publishes `MessageUpdated`; returns the `Message`.

- [ ] **Step 1: Failing tests** (fake provider as in Task 4; helper `wait_for_embeds(app, token, channel, id, n)` polls up to 5 s): `a_sent_link_gets_a_card_over_the_gateway` (socket sees `MessageCreated` without embeds, then `MessageUpdated` with one); `the_first_three_distinct_links_only`; `a_private_ip_link_gets_no_card_and_the_message_is_unchanged`; `editing_adds_new_cards_and_drops_removed_ones`; `an_edit_during_the_fetch_drops_the_stale_card` (fake handler waits on a signal; edit first; then release); `hidden_stays_hidden_through_edits_and_for_everyone`; `only_the_sender_can_hide` (sam → 403); `over_the_unfurl_limit_the_message_posts_without_cards`; `quotes_and_clips_get_no_previews` (quote with a URL in a line: no rows).
- [ ] **Step 2:** Run → fail. **Step 3:** Implement. **Step 4:** Pass.
- [ ] **Step 5:** Commit `feat(embeds): unfurl on send and edit, hide a preview`.

### Task 6: The embed page and `PULSE_PUBLIC_URL`

**Files:** Create `server/src/embeds/page.rs`; modify `server/src/lib.rs`, `deploy/compose.yml`, `deploy/deploy-selftest.sh`, `deploy/README.md` (if it lists env), `SECURITY.md`. Test: `server/tests/embed_page.rs`.

**Interfaces:**
- `GET /embed/{provider}/{id}` (unauthenticated, `?start=N` for YouTube only, digits, at most 86400×7): 404 when `public_url` is None, the provider is unknown/generic, or `valid_id` fails; else 200 `text/html; charset=utf-8`, `Cache-Control: public, max-age=3600`, `X-Content-Type-Options: nosniff`, the Global Constraints' Referrer-Policy and CSP (`frame-src` = the provider player origin), body a static page with one `<iframe src=player_url(…) allow="autoplay; encrypted-media; fullscreen; picture-in-picture" allowfullscreen referrerpolicy="strict-origin-when-cross-origin">`, no scripts. Twitch `parent` = `public_url`'s host.
- compose: `PULSE_PUBLIC_URL: https://${PULSE_DOMAIN:?}` in `pulse-server`'s environment; selftest asserts it.

- [ ] **Step 1: Failing tests:** `the_page_is_off_without_a_public_url`; `each_provider_frames_its_official_player` (exact iframe src per provider, CSP frame-src origin, headers); `invalid_ids_and_unknown_providers_are_404` (`../`, `%2F`, quotes, `<`, too long, `generic`, `vimeo`); `never_redirects` (no 3xx for any input); `start_only_for_youtube_and_only_digits`; `twitch_parent_is_the_public_host`. Config unit: `PULSE_PUBLIC_URL` parse (empty, `https://example.com`, path refused, `ftp:` refused, error names the variable).
- [ ] **Step 2:** Run → fail. **Step 3:** Implement. **Step 4:** Pass; `bash deploy/deploy-selftest.sh` passes.
- [ ] **Step 5:** Commit `feat(embeds): public embed page and PULSE_PUBLIC_URL`.

### Task 7: Client core: hide, embed origin, pop-out, CSP

**Files:** Modify `client/src-tauri/src/api.rs`, `commands.rs`, `lib.rs`, `tauri.conf.json`, `tauri.dev.conf.json`, `tauri.dev.linux.conf.json`, `tauri.linux.conf.json`, `tests/csp.rs`; create `client/src-tauri/src/embed_window.rs`.

**Interfaces:**
- `Api::hide_embed(&self, token: &str, message_id: MessageId, url: Option<&str>) -> Result<Message, ApiError>`; command `hide_embed(message_id, url: Option<String>) -> Message`.
- `pub const EMBED_ORIGIN: &str = "https://pulsechat.co.za";` command `embed_origin() -> Option<String>` – `Some(EMBED_ORIGIN)` only when the signed-in server's origin equals it (`pub fn embed_origin_for(server_url: &str) -> Option<&'static str>`, unit-tested: trailing slash ok, `http://`, other hosts, ports → None).
- `embed_window::popout_url(provider: EmbedProvider, id: &str) -> Option<String>` – Instagram only for now: `https://www.instagram.com/<p|reel>/<code>/embed/` from a validated id (same rules as the server). Command `open_embed_window(provider, id)`: closes an existing `embed` window, builds a new `WebviewWindowBuilder` labelled `embed` (no capability lists it, and Tauri 2.12 refuses IPC from remote origins), same `data_directory` as main, 420×760, title "Pulse – Instagram", `on_navigation` allows only `https:` URLs, `on_new_window` denies.
- CSP: every config's CSP gains `; frame-src https://pulsechat.co.za`; `tests/csp.rs` asserts it in all four and that no other `frame-src`/`child-src` exists. `tests/capabilities.rs` still passes (only `main` has capabilities).

- [ ] **Step 1: Failing unit tests** for `embed_origin_for`, `popout_url`, and the CSP test.
- [ ] **Step 2:** `cargo test -p pulse-client --lib embed` and `--test csp` → fail. **Step 3:** Implement. **Step 4:** Pass (`--lib` and `--test csp --test capabilities`).
- [ ] **Step 5:** Commit `feat(client): embed origin, hide preview, Instagram pop-out, frame-src`.

### Task 8: UI: cards, players, hide, setting

**Files:** Create `client/ui/src/lib/embeds.ts` (+ `.test.ts`), `client/ui/src/lib/linkPreviews.svelte.ts`, `client/ui/src/components/LinkEmbed.svelte` (+ `LinkEmbed.test.ts`); modify `MessageItem.svelte` (+ test), `ChatSettings.svelte`, `lib/tauri.ts`.

**Interfaces:**
- `embeds.ts`: `providerLabel(p): string` (YouTube, TikTok, Spotify, SoundCloud, Twitch, Instagram; generic → `site_name` or the URL's host); `playable(e: LinkEmbed): boolean` (kind player, known provider, id present); `playerSrc(origin: string, e: LinkEmbed): string` → `${origin}/embed/${provider}/${encodeURIComponent(id)}` (+ `?start=N`); `aspect(e): '16 / 9' | '9 / 16' | 'audio'` (Spotify/SoundCloud: fixed 152 px/166 px heights); `SANDBOX: Record<provider, string>` – `allow-scripts allow-same-origin allow-presentation` (no popups) for all player providers; `popsOut(e)`: Instagram.
- `linkPreviews.svelte.ts`: `previews = $state({ show: boolean })` from `localStorage['pulse.linkPreviews']` (default on), `setShowPreviews(v)`; `player = $state({ active: string | null })` with `play(key)`, `stop(key)`; `embedOrigin` loaded once via `api.embedOrigin()`.
- `LinkEmbed.svelte` props `{ embed: LinkEmbed; messageId: string; mine: boolean }`: card (label, title as a link to `embed.url` with `target="_blank" rel="noopener noreferrer"`, description clamped to 2 lines, thumb via `mediaUrl(hash)` or a neutral placeholder for Instagram), a play button over the thumb when `playable`. Click: Instagram → `api.openEmbedWindow`; no `embedOrigin` → open the link (an anchor click); else `play(key)` and the iframe (`src=playerSrc`, `sandbox`, `allow="autoplay; encrypted-media; fullscreen; picture-in-picture"`, `referrerpolicy="strict-origin-when-cross-origin"`, title). Only the active key shows its iframe. An IntersectionObserver (`rootMargin: '400px'`) stops it when it leaves. Sender: a ✕ button "Remove preview" → `api.hideEmbed(messageId, url)`. All text via Svelte text bindings, never `{@html}`.
- `MessageItem.svelte`: under the content, `{#if previews.show}{#each message.embeds as e (e.url)}<LinkEmbed …/>{/each}{/if}` for live, non-pending messages; actions bar gains "Remove previews" (mine and embeds present) → `api.hideEmbed(id, null)`.
- `ChatSettings.svelte`: a "SHOW LINK PREVIEWS" `Segmented` (On/Off) bound to `previews.show`.
- `tauri.ts`: `hideEmbed(messageId, url|null)`, `embedOrigin()`, `openEmbedWindow(provider, id)`.

- [ ] **Step 1: Failing tests:** `embeds.test.ts` (labels, `playerSrc` encoding and start, aspect, `playable` false for generic/unknown/missing id). `LinkEmbed.test.ts`: renders title/description containing `<img src=x onerror=…>` as text (no `img` element besides the thumb); clicking play swaps the thumb for an iframe whose src starts with `https://pulsechat.co.za/embed/youtube/`; starting a second player removes the first iframe; without an origin the play control is a link to `embed.url` and no iframe appears; Instagram calls `open_embed_window`; ✕ shown only when `mine` and calls `hide_embed`. `MessageItem.test.ts`: embeds render; with the setting off none render; "Remove previews" only on your own message.
- [ ] **Step 2:** `nice -n 19 pnpm test` → fail. **Step 3:** Implement. **Step 4:** `pnpm check` 0/0 and `pnpm test` pass.
- [ ] **Step 5:** Commit `feat(ui): link preview cards, click-to-play players, hide and setting`.

### Task 9: Whole-branch checks

- [ ] `git merge main` if main moved; then the final checks: fmt, clippy (Linux + xwin per CI), `cargo test -p pulse-server`, `cargo test -p pulse-client --lib`, `cargo deny check`, bindings with no diff, `pnpm install --frozen-lockfile && pnpm check && pnpm test`, `bash deploy/deploy-selftest.sh`.
- [ ] Fresh reviewer over the branch diff; fix Critical/Important test-first; commit.
