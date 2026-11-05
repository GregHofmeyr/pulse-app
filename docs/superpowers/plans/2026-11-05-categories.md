# Channel categories and drag to reorder — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Named categories that hold channels of every kind, replacing the four fixed sidebar sections, with
drag-to-reorder (and Alt+arrow / Move up / Move down) of channels and categories, saved for everyone.

**Architecture:** The server owns the layout: a `categories` table, `channels.category_id`, one position order per
category (and one for the uncategorised area), and a per-server `layout_revision` bumped by every layout change. Every
change publishes the server's whole layout (`LayoutChanged`, everyone, like `ChannelDeleted`); the one layout route
takes the whole order plus the revision it was based on and refuses (409) a stale or incomplete one. The client
keeps layouts in the pure reducer, renders through one pure selector (`lib/layout.ts`), shows an optimistic draft
while a save is in flight (`lib/layoutDraft.svelte.ts`) and drags with hand-built pointer events (`lib/drag.ts`,
transforms only).

**Tech Stack:** Rust (axum, sqlx/SQLite, ts-rs), Tauri 2 core, Svelte 5 + vitest.

**Spec:** `docs/superpowers/specs/2026-11-05-categories-design.md` (issues #54, #55 reorder part).

## Global Constraints

- Public repo: no IPs, secrets, emails or real usernames; test people are alex, sam, kim, jo.
- Theme tokens only (`var(--…)`), `var(--radius-*)`, motion tokens (`--dur-fast`, `--dur`, `--dur-slow`,
  `--dur-exit`, `--ease`); guard tests `motionCss.test.ts`, `colorTokens.test.ts`, icon-size guards must stay green.
  Nothing transitions layout properties or box-shadow; drag moves with `transform` only.
- Protocol additions are `serde(default)`; optional TS fields `#[ts(optional)]` (Vec in Ready:
  `#[ts(as = "Option<_>", optional)]`), so old clients and existing fixtures keep working.
- Migration number **0023** (`server/migrations/0023_categories.sql`).
- Default category names exactly: "Text Channels", "Quotes", "Clips", "Voice Channels", in that order.
- Category names 1–64 chars via `clean_name`; cap **50** categories per server (`caps::CATEGORIES_PER_SERVER`).
- Who may change: server members only (403 otherwise); non-members see categories, cannot drag or open menus.
- Drag starts after **4px** of movement; Escape cancels; hovering a collapsed category **600 ms** while dragging opens it.
- Headings look exactly like today's (`.label` / `.section` styles, `--text-3`, chevron), name upper-cased by CSS
  (`text-transform: uppercase`), stored as typed.
- Every cargo command: `CARGO_BUILD_JOBS=4 flock ~/Projects/pulse-app/target/.cargo-serial nice -n 19 ionice -c3 cargo …`
  (this worktree's own `target/`), tests via `cargo t … -- --test-threads=4`. pnpm without nice; node from
  `~/.config/nvm/versions/node/v22.22.3/bin`.
- Commits reference #54 / #55 and end with the Co-Authored-By / Claude-Session lines.

## Review Focus

1. Two people reorder at the same moment: the second save is refused (409) and its client snaps to the first's order
   without a flash. Test: `a_stale_layout_is_refused` (server), `a refused save snaps back` (client).
2. A rename's `ChannelUpdated` racing a layout change must not undo the move on clients. Test:
   `ChannelUpdated keeps the placement the layout gave` (state.test.ts).
3. A `LayoutChanged` arriving out of order (older revision after newer) is ignored. Test:
   `an older LayoutChanged is ignored` (state.test.ts).
4. A layout naming a soft-deleted channel, a channel twice, or another server's category is refused (409), never
   half-applied. Test: `an_incomplete_or_foreign_layout_is_refused`.
5. Collapsed "Voice Channels" from before the update stays collapsed (carry-over by `default_kind`, even after a
   rename). Test: `old collapsed sections carry over to their default categories`.

---

## File Structure

| File | Responsibility |
|---|---|
| `protocol/src/ids.rs` | `CategoryId` |
| `protocol/src/rest.rs` | `Category`, `ChannelPlacement`, `ServerLayout`, `CreateCategoryRequest`, `RenameCategoryRequest`, `SaveLayoutRequest`, `LayoutCategory`; `Channel.category_id`; `CreateChannelRequest.category_id` |
| `protocol/src/gateway.rs` | `Ready.layouts`; `Event::LayoutChanged { layout }` |
| `server/migrations/0023_categories.sql` | table, columns, defaults for existing servers |
| `server/src/servers/categories.rs` | layout read/bump/publish, category routes, layout route |
| `server/src/servers/{routes,channels}.rs`, `server/src/access.rs` | category_id in rows; defaults for new servers; create in a category; bumps on create/delete/restore |
| `server/src/limits.rs`, `gateway/socket.rs` | `layout_user` rate, cap; Ready layouts |
| `server/tests/categories.rs` | migration + route + event tests |
| `client/src-tauri/src/{api,commands,lib}.rs` | category/layout calls and commands |
| `client/ui/src/lib/state.ts` | `layouts` slice, `applyLayout`, `LayoutChanged`, placement-preserving `ChannelUpdated` |
| `client/ui/src/lib/layout.ts` | pure: `sidebarLayout`, `orderOf`, moves, `applyOrder`, `toRequest` |
| `client/ui/src/lib/layoutDraft.svelte.ts` | optimistic draft + `saveOrder` |
| `client/ui/src/lib/sidebarSections.svelte.ts` | collapse by key; carry-over |
| `client/ui/src/lib/categoryUi.svelte.ts` | "Create category" request from menus to the list |
| `client/ui/src/lib/drag.ts` | pointer-drag controller (geometry cached at start, transforms only) |
| `client/ui/src/components/ChannelList.svelte` | uncategorised then categories, inline create (kind picker), rename, drag, Alt+arrows |
| `client/ui/src/components/CategoryMenu.svelte` | heading right-click menu |
| `ChannelMenu.svelte`, `ServerMenu.svelte`, `routes/Shell.svelte` | Move up/down; Create category |

---

### Task 1: Protocol

**Files:** Modify `protocol/src/ids.rs`, `protocol/src/rest.rs`, `protocol/src/gateway.rs`; regenerate
`client/ui/src/lib/protocol/*` (`cargo t -p pulse-protocol export_bindings`); fix Rust struct literals of `Channel`,
`CreateChannelRequest`, `Ready` (server src+tests, client core) until `cargo check --workspace --all-targets` is clean.

**Interfaces — Produces:**
- `id!(CategoryId)`.
- `Category { id: CategoryId, server_id: ServerId, name: String, position: i64 (ts number), default_kind: Option<ChannelKind> }`
  — `default_kind`: `serde(default, skip_serializing_if = "Option::is_none")`, `ts(optional)`; the kind a default
  category was made for (migration / new server), `None` for categories people make.
- `ChannelPlacement { channel_id: ChannelId, category_id: Option<CategoryId>, position: i64 }`
- `ServerLayout { server_id: ServerId, revision: i64, categories: Vec<Category>, channels: Vec<ChannelPlacement> }`
  — categories in order; `channels` `serde(default)`: every live channel's placement (empty in Ready, where the
  channels carry their own).
- `Channel.category_id: Option<CategoryId>` — `serde(default, skip_serializing_if = "Option::is_none")`, `ts(optional)`.
  `position` now means the order inside its category (or the uncategorised area).
- `CreateChannelRequest.category_id: Option<CategoryId>` — same attributes.
- `CreateCategoryRequest { name: String }`, `RenameCategoryRequest { name: String }`
- `SaveLayoutRequest { revision: i64, uncategorised: Vec<ChannelId>, categories: Vec<LayoutCategory> }`,
  `LayoutCategory { id: CategoryId, channels: Vec<ChannelId> }`
- `Ready.layouts: Vec<ServerLayout>` — `serde(default)`, `ts(as = "Option<_>", optional)`.
- `Event::LayoutChanged { layout: ServerLayout }` — `channel_id()` `None` (everyone, as `ChannelDeleted`), `only_for()` `None`.

- [ ] Tests in `protocol`: `a_channel_without_a_category_parses` (old JSON → `category_id: None`, and serialising it
  leaves the key out), `a_ready_without_layouts_parses`, `layout_changed_goes_to_everyone` (`channel_id()` and
  `only_for()` are `None`), `create_channel_without_a_category_parses`.
- [ ] Fail → implement → pass; export bindings; fix literals (`category_id: None`, `layouts: vec![]`); commit.

### Task 2: Migration 0023 and default categories for new servers

**Files:** Create `server/migrations/0023_categories.sql`, `server/tests/categories.rs`. Modify
`server/src/access.rs` (`CHANNEL_COLS` + `ChannelRow` + `channel_from_row` gain `category_id`; also the
`DeletedRow` users in `servers/channels.rs`), `server/src/servers/routes.rs` (`create_server`, `insert_channel`).

**Schema:**
```sql
ALTER TABLE servers ADD COLUMN layout_revision INTEGER NOT NULL DEFAULT 0;
CREATE TABLE categories (
    id           TEXT PRIMARY KEY,
    server_id    TEXT NOT NULL REFERENCES servers(id) ON DELETE CASCADE,
    name         TEXT NOT NULL,
    position     INTEGER NOT NULL,
    default_kind TEXT CHECK (default_kind IN ('text', 'quotes', 'clips', 'voice')),
    created_at   TEXT NOT NULL
) STRICT;
CREATE INDEX categories_server ON categories(server_id, position);
ALTER TABLE channels ADD COLUMN category_id TEXT REFERENCES categories(id) ON DELETE SET NULL;
CREATE INDEX channels_category ON channels(category_id);
```
Data, in the same file:
- One category per (server, kind ∈ text, quotes, clips, voice) that has at least one **live** channel; names
  "Text Channels", "Quotes", "Clips", "Voice Channels"; `position` = rank among that server's defaults in that kind
  order (0-based, `ROW_NUMBER()`); `default_kind` = the kind; `created_at` =
  `strftime('%Y-%m-%dT%H:%M:%fZ', 'now')`.
- The category's id is the smallest live channel id of its (server, kind): a valid ULID, unique among categories
  (each channel id is the minimum of at most one group). Comment why.
- Every channel (live and soft-deleted) of that (server, kind) gets that `category_id`; soft-deleted channels of a
  kind without a category stay `NULL` (a restore puts them at the bottom of the uncategorised area).
- Live channels' `position` = `ROW_NUMBER() OVER (PARTITION BY server_id, kind ORDER BY position, name, id) - 1`
  (today's sidebar order: position, then name). Soft-deleted positions are left as they were.
- End with the scoped `pragma_foreign_key_check` guard 0008 uses.

**New servers:** `create_server` makes "Text Channels" (`default_kind` text, position 0) holding `general` and
"Voice Channels" (voice, position 1) holding `Lounge`. `insert_channel` gains `category: Option<CategoryId>`.

- [ ] Tests in `server/tests/categories.rs` (migration in the `names.rs` style: run migrations `< 23`, seed, run all):
  - `migration_0023_puts_every_channel_in_its_default_category_in_order` — two servers: "Main" with text
    `general`(0) `memes`(1) `news`(2, inserted before memes to prove ordering is by position), quotes `quotes`,
    clips `clips`, voice `Lounge`(0) `Games`(1), and a soft-deleted text `old`; "Side" with only text `chat` and
    voice `Hangout`. After: Main has 4 categories named and ordered as above with `default_kind`s, Side has 2
    ("Text Channels" pos 0, "Voice Channels" pos 1); each live channel's category matches its kind and positions are
    0..n in old order; `old` is in Main's "Text Channels"; `layout_revision` 0; `PRAGMA integrity_check` = "ok",
    `PRAGMA foreign_key_check` empty; no channel lost (count before = after).
  - `a_kind_without_channels_gets_no_category` — Side has no Quotes/Clips category; a server whose only quotes
    channel is soft-deleted gets no Quotes category and that channel's `category_id` is NULL.
  - `a_new_server_starts_with_text_and_voice_categories` — via the API: Ready / `GET /servers/{id}/channels` show
    `general` in "Text Channels" and `Lounge` in "Voice Channels", positions 0.
- [ ] Fail → implement → pass; full server suite still green; commit.

### Task 3: Category and layout routes, revision, events

**Files:** Create `server/src/servers/categories.rs`; modify `server/src/servers/{mod,routes,channels}.rs`,
`server/src/limits.rs`, `server/src/gateway/socket.rs`, `server/src/lib.rs` (router); tests in
`server/tests/categories.rs`.

**Interfaces — Produces (in `servers/categories.rs`):**
- `pub async fn layout_of(db: &mut SqliteConnection, server: ServerId) -> AppResult<ServerLayout>` — categories by
  `position, id`; live channels' placements by `category, position, id`.
- `pub async fn bump(tx: &mut SqliteConnection, server: ServerId) -> AppResult<i64>` — `layout_revision + 1`, returned.
- `pub async fn publish_layout(s: &AppState, server: ServerId)` — reads `layout_of` and publishes `LayoutChanged`
  (a failed read is logged; the change has landed).
- `pub async fn all_layouts(db: &SqlitePool) -> AppResult<Vec<ServerLayout>>` — every server's, `channels` empty; Ready.
- `pub async fn bottom_of(tx, server, category: Option<CategoryId>) -> AppResult<i64>` — `MAX(position)+1` of live
  channels there (0 when empty).
- Routes: `POST /servers/{id}/categories` (`CreateCategoryRequest` → `Category`, at the bottom),
  `PATCH /categories/{id}` (`RenameCategoryRequest` → `Category`), `DELETE /categories/{id}` → 204 (its live
  channels are appended, in their order, to the bottom of the uncategorised area; soft-deleted ones lose the
  category by `ON DELETE SET NULL`), `PUT /servers/{id}/layout` (`SaveLayoutRequest` → `ServerLayout`).
- Layout route: 409 `Conflict("The order changed just now, so it's back to how it is for everyone.")` unless
  `revision` is current AND the categories named are exactly the server's (each once) AND the channels named
  (uncategorised + all categories) are exactly its live channels (each once). On success: positions 0..n per list,
  `category_id` per list, bump, publish. All writes inside one `begin_write` transaction.
- Every route: `limits.layout_user` (`Rate::new(30, 1.0)`, doc "generous, like renames") → 429; member of the server
  (403; 404 for no such server/category); category cap `caps::CATEGORIES_PER_SERVER = 50` → 400.
- Every layout-changing write (the four routes, `create_channel`, channel `delete`, `restore`) holds
  `s.hub.in_order()` from before `begin_write` until `publish_layout` returns, bumps the revision in its transaction,
  and calls `publish_layout` after its own event. `create_channel` takes `category_id` (400 unless it is a category
  of that server; else appended to the bottom of that category, `None` → bottom of uncategorised). `restore` puts the
  channel at the bottom of its category if that still exists, else at the bottom of the uncategorised area.
- Ready: `layouts: all_layouts(db)`.

- [ ] Tests (sockets via `app.hub.register(user, String::new()).rx`, world: alex owns "Main", sam joined, jo not in it):
  - `members_create_rename_and_delete_categories_and_everyone_hears` — sam creates "Gaming" → 200, position after the
    defaults; jo (non-member) gets `LayoutChanged` whose categories end with Gaming and revision went up; rename →
    "Games" shows in the next `LayoutChanged`; jo's create/rename/delete → 403.
  - `deleting_a_category_moves_its_channels_to_the_top_area_in_order` — channels `a`,`b` in Gaming, `x` already
    uncategorised → after delete, uncategorised order is `x, a, b`; no channel deleted.
  - `a_layout_saves_and_reaches_everyone` — move `general` into Gaming at index 0 and Gaming above "Text Channels":
    200 with the new layout; `GET /servers/{id}/channels` shows the new `category_id`/positions; jo gets it.
  - `a_stale_layout_is_refused` — two saves with the same revision: first 200, second 409; order is the first's.
  - `an_incomplete_or_foreign_layout_is_refused` — missing a channel, a channel twice, a soft-deleted channel, a
    category of another server, a missing category → each 409, nothing changed (revision unchanged).
  - `non_members_cant_save_a_layout` — jo → 403.
  - `channels_are_created_inside_a_category` — `category_id` Gaming → lands last in Gaming; another server's category
    → 400; no category → bottom of uncategorised; each publishes `ChannelCreated` then `LayoutChanged`.
  - `create_delete_and_restore_bump_the_revision` — each raises Ready's revision for the server by one.
  - `a_channel_restored_into_a_deleted_category_lands_at_the_bottom_of_the_top_area` — and one restored into a living
    category lands at its bottom.
  - `the_category_cap_holds` (50) and `layout_changes_are_rate_limited` (`layout_user = Rate::new(2, 0.0001)`: the
    third → 429).
  - `ready_carries_every_servers_layout`.
- [ ] Fail → implement → pass; whole server suite; commit.

### Task 4: Client core — category and layout commands

**Files:** `client/src-tauri/src/{api,commands,lib}.rs`.

**Interfaces — Produces:** `Api::create_channel_in(&self, token, server, kind, name, category: Option<CategoryId>)`
(the old `create_channel` calls it with `None`), `Api::create_category(token, server, name) -> Category`,
`Api::rename_category(token, CategoryId, name) -> Category`, `Api::delete_category(token, CategoryId) -> ()`,
`Api::save_layout(token, server, &SaveLayoutRequest) -> ServerLayout`; commands `create_channel { serverId, kind,
name, categoryId?: string|null }`, `create_category { serverId, name }`, `rename_category { categoryId, name }`,
`delete_category { categoryId }`, `save_layout { serverId, layout }`, registered in `lib.rs`.

- [ ] Test `categories_round_trip_through_the_core` in `api.rs` (spawned test server): create → rename → create a
  channel in it → save a layout moving it → stale save is an error → delete. Fail → implement → pass → commit.

### Task 5: Client state, selector, moves, draft, collapse

**Files:** `client/ui/src/lib/{state,layout,layoutDraft.svelte,sidebarSections.svelte,categoryUi.svelte,tauri}.ts`
(+ `layout.test.ts`, `layoutDraft.svelte.test.ts`, `sidebarSections.test.ts`, `state.test.ts`).

**Interfaces — Produces:**
- `AppState.layouts: Record<string, { revision: number; categories: Category[] }>` (server id → layout; empty from an
  older server). `applyReady` fills it from `r.layouts ?? []`.
- `applyLayout(s: AppState, layout: ServerLayout): AppState` — ignored when `layout.revision` < held revision; sets
  the layout and each known channel's `category_id`/`position` from `layout.channels` (same object when unchanged).
  `LayoutChanged` → `applyLayout`. `ChannelUpdated` keeps the held channel's `category_id` and `position`.
- `lib/layout.ts`:
  - `type Section = { key: string; category: Category | null; kind: ChannelKind | null; name: string; channels: Channel[] }`
  - `type SidebarLayout = { live: boolean; revision: number; top: Channel[]; sections: Section[] }`
  - `sidebarLayout(s, serverId)` — live: `top` = channels with no (or an unknown) category, then one section per
    category (key `cat:<id>`), each by `position` then name. Not live (older server): `top` empty and the four
    sections of today (keys `<serverId>:text|quotes|clips|voice`, names "Text Channels", "Quotes", "Clips",
    "Voice Channels", `kind` set, `category` null) — always all four.
  - `type Order = { top: string[]; cats: { id: string; channels: string[] }[] }`; `orderOf(l)`, `applyOrder(l, o)`
    (re-sorts `l` by `o`; channels/categories not in `o` keep their place at the end), `toRequest(o, revision)`.
  - `moveChannel(o, channelId, to: { category: string | null; index: number }): Order`,
    `moveCategory(o, categoryId, index): Order`,
    `stepChannel(o, channelId, dir: -1 | 1): Order | null` (at a list's edge it goes to the end of the previous
    list or the start of the next: top ↔ first category ↔ … ; null at the very top/bottom),
    `stepCategory(o, categoryId, dir): Order | null`.
- `lib/layoutDraft.svelte.ts`: `drafts: { byServer: Record<string, Order> }` (`$state`), `saveOrder(serverId: string,
  order: Order): Promise<void>` — sets the draft, `api.saveLayout(serverId, toRequest(order, revision))`; success:
  `app.state = applyLayout(app.state, result)` then drop the draft (no flash); refusal: drop the draft (snap back)
  and `ui.notice = errorText(e)`. A second save while one is in flight replaces the draft and is based on the same
  held revision (it will be refused if the first landed; acceptable).
- `sidebarSections.svelte.ts`: `isCollapsed(key)`, `toggleSection(key)`, `setCollapsed(key, on)`;
  `carryOverCollapsed(layouts: ServerLayout[])` replaces each saved `<serverId>:<kind>` with `cat:<id>` of that
  server's category whose `default_kind` is that kind (when there is one), persisting; registered with `onAppReady`.
- `lib/categoryUi.svelte.ts`: `categoryUi: { create: { serverId: string; seq: number } | null }`,
  `requestCreateCategory(serverId)`.
- `tauri.ts`: `createChannel(serverId, kind, name, categoryId?: string | null)`, `createCategory(serverId, name)`,
  `renameCategory(categoryId, name)`, `deleteCategory(categoryId)`, `saveLayout(serverId, layout: SaveLayoutRequest)`.

- [ ] Tests: `state.test.ts` — `LayoutChanged moves channels and categories`, `an older LayoutChanged is ignored`,
  `ChannelUpdated keeps the placement the layout gave`, `a Ready without layouts is a legacy layout`;
  `layout.test.ts` — sidebarLayout live and legacy, every move (within, between, out of a category, into an empty
  one, a category up/down), steps at edges, `applyOrder` ∘ `orderOf` is identity, `toRequest`;
  `layoutDraft.svelte.test.ts` — `the draft shows at once and the saved layout replaces it`, `a refused save snaps
  back`; `sidebarSections.test.ts` — `old collapsed sections carry over to their default categories` (incl. a
  renamed one), unknown keys left alone. Fail → implement → pass → commit.

### Task 6: ChannelList by categories, menus, inline create/rename, keyboard moves

**Files:** `client/ui/src/components/{ChannelList,CategoryMenu,ChannelMenu,ServerMenu}.svelte`,
`routes/Shell.svelte`; tests `ChannelList.test.ts`, `ChannelListCollapse.test.ts`, `CategoryMenu.test.ts`,
`ChannelMenu.test.ts`, `ServerMenu.test.ts`.

**Behaviour:**
- List: `top` rows (no heading), then each section `<div class="cat" data-category=…>` with today's heading markup
  (`.label` > `button.section` + chevron + `.add`); name upper-cased by CSS. Heading top padding as today: 12px when
  it is the first thing in the list, else 16px. Row markup unchanged per kind (text/quotes/clips row; voice row with
  occupants).
- Collapsed section keeps: the open channel, unread unmuted chat channels, and the voice channel you're in (with its
  occupants); other occupied voice channels hide.
- `+` on a live category opens an inline field under the heading: `.input.input-sm` plus a small kind picker
  (radiogroup of four icon buttons `hash`, `quote`, `film`, `speaker`, labels "Text", "Quotes", "Clips", "Voice",
  default Text, styled as `.add` with the chosen one `aria-checked` and `var(--bg-3)` / `var(--text)`); Enter creates
  in the category (`tidyName` as today), Escape or focus leaving the field-and-picker cancels. Legacy sections keep
  today's per-kind `+`.
- Category heading right-click / Shift+F10 (members, live only) → `CategoryMenu`: "Create channel", "Rename
  category…", "Delete category" (danger), separator, "Move up", "Move down" (disabled at the ends). Rename edits the
  heading inline (as channel rename). Delete has no dialog.
- `ChannelMenu` gains, for members on a live layout, "Move up" / "Move down" (props `onMoveUp`, `onMoveDown`,
  `canMoveUp`, `canMoveDown`) after a separator.
- Alt+ArrowUp / Alt+ArrowDown on a focused channel row or category heading → `stepChannel` / `stepCategory` →
  `saveOrder`; focus stays on the moved row/heading.
- "Create category": `ServerMenu` (members only) and the sidebar header menu in `Shell.svelte` (members, live) →
  `requestCreateCategory(serverId)`; the tab menu also switches to that server. ChannelList shows a heading-styled
  inline field at the end of the list for it; Enter → `api.createCategory`.

- [ ] Tests: renders top rows then categories in order with their names; heading text is the stored name and the
  label has `text-transform: uppercase`; collapse per category persists across remount and keeps the active,
  unread and joined-voice rows; `+` creates each kind inside its category (`createChannel` called with the category
  id); category menu rename/delete/create-channel; Move up/down from both menus; Alt+arrows; non-members get no
  menu, no `+`, no Alt move; Create category from ServerMenu and the header menu; legacy (no layouts) still shows
  the four sections with per-kind `+`. Update existing `ChannelList*.test.ts` fixtures for the new heading text.
  `pnpm check`, `pnpm test`. Commit.

### Task 7: Drag to reorder

**Files:** Create `client/ui/src/lib/drag.ts` (+ `drag.test.ts`); modify `ChannelList.svelte` (+ `ChannelListDrag.test.ts`).

**Interfaces — Produces:** `sortableList(list: HTMLElement, opts: { enabled: () => boolean; onDrop: (d: Drop) => void;
onHoverSection: (key: string) => void })` → `{ destroy() }` (a Svelte action), where
`type Drop = { kind: 'channel'; id: string; to: { category: string | null; index: number } } | { kind: 'category'; id: string; index: number }`.
Pure helpers exported for tests: `slotFor(items: Rect[], y: number): number`, `shiftsFor(fromIndex, toSlot, height, count): number[]`.

**Behaviour:** pointerdown (button 0) on a `.row[data-channel]` or a heading (`[data-category] > .label`); past 4px
of movement the drag starts: pointer capture, all item rects measured **once**; per frame (`requestAnimationFrame`)
only `transform: translate3d(0, …, 0)` on the lifted item (class `lifted`: `box-shadow: var(--shadow-1)`,
`background: var(--bg-2)`, above siblings) and on the items making room (`transition: transform var(--dur)
var(--ease)`), plus a 2px `var(--accent)` drop line positioned by transform. A category drags as its whole section
and drops between sections (after the top area). Hovering a collapsed section 600 ms → `onHoverSection` (opens it;
rects re-measured once). Escape → everything eases back. Drop → `onDrop`; ChannelList turns it into `moveChannel` /
`moveCategory` → `saveOrder`; transforms are cleared in the same frame the new order renders (class `settling`
disables transitions for that frame), so nothing jumps or flashes. The click that ends a drag is swallowed; a plain
click and right-click behave as before. Reduced motion: the global rule in `app.css` already zeroes durations.

- [ ] Tests: `drag.test.ts` (slot and shift maths); `ChannelListDrag.test.ts` with stubbed `getBoundingClientRect`
  and pointer events: drag within a category, into another, into the top area, into an empty category, a category
  above another (each calls `saveOrder` with the expected order); under 4px it is a click (`onSelect` called, no
  save); Escape cancels (no save); non-members can't drag. Guard tests stay green. Commit.

### Task 8: Verify, rehearse, issues, as-built notes

- [ ] Rehearse 0023 on a realistic file DB with `sqlite3` (migrations 0001–0022, seeded servers with text, quotes,
  clips, voice and a soft-deleted channel; then 0023; `pragma integrity_check`, `pragma foreign_key_check`, per
  channel category and order vs before).
- [ ] fmt, clippy `-D warnings`, `cargo xwin clippy -p pulse-client --target x86_64-pc-windows-msvc --all-targets --
  -D warnings`, server + protocol + client tests, bindings check, `pnpm check`, `pnpm test`.
- [ ] Issues for deferrals; "As built" notes here; final commit `Closes #54`.
