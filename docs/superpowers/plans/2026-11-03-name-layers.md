# Name Layers and Nickname Renaming Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Display names, server nicknames anyone can set (recorded in the Name Archive), private nicknames, and one
client name function used everywhere (#47, #48).

**Architecture:** Server adds `users.display_name`, a `private_nicknames` table, a server-nickname rename route that
writes `nickname_history` in the same transaction and publishes `MemberUpdated`, a private-nickname route whose event
and Ready field only ever reach the owner, and a paged archive route. The client keeps the precedence in
`displayName` (`lib/selectors.ts`) and routes every rendered name through it. Mentions stay `@username` on the wire
(the server's parser, old clients and search keep working); the composer shows them as chips and messages as pills
labelled with the shown name.

**Tech Stack:** Rust (axum, sqlx/SQLite, ts-rs), Tauri 2 core, Svelte 5 + vitest.

**Spec:** `docs/superpowers/specs/2026-11-03-name-layers-design.md`

## Global Constraints

- Display name and nicknames: 1–32 characters after trimming; no line breaks or control characters; not unique.
- Precedence — in a server: server nickname, else private nickname, else display name. Elsewhere: private nickname,
  else display name. The username shows only on the profile card (`@username`).
- Renames are quiet (no system line); every server-nickname change (reset included) writes one `nickname_history` row.
- Rate limit: 30 renames a minute per renamer (`limits.rename_user`), refused with 429 and words.
- Private nicknames never appear in another user's Ready, events or REST payloads.
- Migration is **0013** (renumber at merge if main gained one); existing users get `display_name = username`.
- New protocol fields `#[serde(default)]`. Test people alex/sam/kim/jo. Builds via the flock/nice throttle.

## Review Focus

1. A private nickname set by alex reaching sam (Ready, `PrivateNicknameChanged`, profile, members) — test in Task 2.
2. A name made only of spaces / a newline / 33 emoji-free chars — rejected with words (Task 1 unit tests).
3. A user from an old server (no `display_name`, i.e. `''`) — client falls back to username (Task 4 test).
4. Typing `@alex` by hand (the username) renders as the shown name pill (Task 5 test).
5. Renaming someone who is not in the server, or renaming while not a member → 404 / 403 (Task 2 test).

---

### Task 1: Server — migration 0013, display name, validation

**Files:** Create `server/migrations/0013_name_layers.sql`, `server/src/profiles/names.rs`. Modify
`protocol/src/rest.rs` (`User`, `UpdateProfileRequest`), `server/src/profiles/users.rs` (`USER_COLS`, `UserRow`),
`server/src/profiles/{mod,routes}.rs`, `server/src/auth/routes.rs` (register inserts `display_name`). Test
`server/tests/names.rs`, unit tests in `names.rs`.

**Interfaces — produces:**
- `User.display_name: String` (`serde(default)`); `UpdateProfileRequest.display_name: Option<String>` (`serde(default)`;
  `None` = keep).
- `profiles::names::clean(s: &str) -> AppResult<String>` (1–32 chars after trim, no control chars; error text
  "Names can be 1 to 32 characters." / "Names can't contain line breaks or control characters.").
- `profiles::names::clean_nickname(s: Option<String>) -> AppResult<Option<String>>` (blank → `None`).

- [ ] Tests: `clean` accepts `"Alex 🎸"`, trims, rejects `""`, `"   "`, 33 chars, `"a\nb"`, `"a\u{7}"`; accepts 32.
- [ ] Integration: `migration_0013_copies_usernames` (pre-0013 db with alex → `display_name = 'alex'`, `private_nicknames`
  exists); `new_users_get_their_username_as_display_name`; `display_name_saves_and_reaches_everyone` (PUT /me/profile
  with `"Alex 🎸"` → profile + `ProfileUpdated.user.display_name`); `display_name_missing_keeps_it`;
  `bad_display_name_is_400`.
- [ ] Implement, run, commit (`feat(server): display names (#47)`).

### Task 2: Server — server nicknames, private nicknames, archive route

**Files:** Create `server/src/servers/nicknames.rs` (router merged in `lib.rs`), `server/src/profiles/private.rs`.
Modify `protocol/src/{rest,gateway}.rs`, `server/src/limits.rs`, `server/src/gateway/socket.rs` (Ready),
`server/src/profiles/mod.rs` (embedded archive capped). Test `server/tests/names.rs`.

**Interfaces — produces:**
- `rest::SetNicknameRequest { nickname: Option<String> }`; `rest::PrivateNickname { user_id: UserId, nickname: String }`;
  `rest::NameChange { name: Option<String>, changed_by: UserId, changed_at: String }`;
  `rest::NameArchivePage { entries: Vec<NameChange>, total: u32, next_cursor: Option<String> }`.
- `Ready.private_nicknames: Vec<PrivateNickname>` (`serde(default)`).
- `Event::MemberUpdated { server_id, member }` (everyone); `Event::PrivateNicknameChanged { user_id, target_id,
  nickname: Option<String> }` (`only_for` = `user_id`).
- `PUT /servers/{sid}/members/{uid}/nickname` → `Member` (403 caller not a member, 404 target not a member, 429 over
  limit "You're renaming too fast. Try again in a minute."). No-op rename writes nothing.
- `GET /servers/{sid}/members/{uid}/names?cursor=` → `NameArchivePage` (newest first, 50 a page, `total` = all rows
  incl. resets; 403 for non-members). Cursor = opaque `changed_at|id`.
- `PUT /me/private-nicknames/{uid}` → 204 (404 unknown user, 400 yourself). Uses `limits.rename_user` too.

- [ ] Tests: `anyone_can_rename_anyone_and_it_is_archived` (sam renames alex, alex renames self, kim resets → 3 rows,
  old/new recorded, `MemberUpdated` reaches jo's socket, no system message in #general);
  `rename_needs_membership` (403/404); `rename_rate_limit` (limit 2/min config → third is 429 with words);
  `same_name_is_a_noop`; `archive_pages_newest_first` (55 renames → 50 + cursor → 5, total 55, outsider 403);
  `private_nickname_reaches_only_its_owner` (alex sets "Sammy" for sam: alex's Ready has it, alex's socket gets the
  event; sam's and kim's Ready lack it, their sockets get nothing within 300 ms after a later marker event; no payload
  sam can fetch contains "Sammy" — profile, members, Ready JSON); `private_nickname_clear_and_rules` (clear → gone,
  self → 400, unknown → 404, too long → 400).
- [ ] Implement, run, commit (`feat(server): server and private nicknames, Name Archive route (#48)`).

### Task 3: Server — names the server renders

**Files:** Modify `server/src/dms/routes.rs`, `server/src/servers/routes.rs` (group system lines use display name),
`server/src/quotes/{mod,routes}.rs` (speaker label = nickname else display name; quote search also matches display
name and nickname). Test in `server/tests/names.rs`.

- [ ] Tests: `group_system_lines_use_display_names`; `quote_search_finds_display_names_and_nicknames`.
- [ ] Implement, run, commit (`feat(server): server-rendered names use display names (#47)`).

### Task 4: Client core + state + the name function

**Files:** Modify `client/src-tauri/src/{api,commands,lib}.rs` (`set_nickname`, `set_private_nickname`,
`name_archive`), `client/ui/src/lib/{tauri,state,selectors}.ts`, regenerated `client/ui/src/lib/protocol/*`.

**Interfaces — produces:**
- `api.setNickname(serverId, userId, nickname: string | null): Promise<Member>`,
  `api.setPrivateNickname(userId, nickname: string | null): Promise<void>`,
  `api.nameArchive(serverId, userId, cursor: string | null): Promise<NameArchivePage>`.
- `AppState.privateNicks: Record<string, string>` (target → nickname).
- `baseName(u: User): string` (`display_name || username`); `displayName(s, serverId, userId)` implements precedence.
- `mentionablesFor(s, channelId): Mentionable[]` with `Mentionable = { id; name (shown); username; alts: string[] }`.

- [ ] Tests (selectors/state): precedence in a server, in a DM/group (`serverId` null), with no membership, for me,
  old-server user with `display_name: ''`; `MemberUpdated` replaces the member; `PrivateNicknameChanged` sets/clears;
  Ready loads `private_nicknames`; core round-trip test in `api.rs`.
- [ ] Implement, run, commit.

### Task 5: Client — mentions, every name call site, search

**Files:** `lib/{mentions,mentionStyles,composerDom,notify,conversations}.ts`, `components/EmojiBox.svelte`,
`Composer.svelte`, `MessageItem.svelte`, and every site from `grep -rnE "displayName\(|\.username|nameOf\("`:
SpeakerLinkSettings, QuoteItem, QuoteComposer, ProfileCard(Popover), FullProfile, GroupMembers, TextChannel,
SearchBox, MemberList, PeopleBoard, PeoplePicker, ConversationList, Shell, ProfileSettings.

- [ ] Tests: `suggest` matches username/display/nickname/private and returns the shown name; `applyMention` inserts
  `@username`; composer chip shows `@Shown` and serializes to `@username`; `styleMentions` labels a typed `@alex` pill
  with the shown name; `showMentions` rewrites plain text (notification body); SearchBox finds a person by any of the
  four names; MemberList shows nickname with the display name small, never the username.
- [ ] Implement, run, commit.

### Task 6: Client — UI for renaming, private nicknames, display name, archive

**Files:** `components/ProfileSettings.svelte` (+`lib/profileDraft.ts`), `ProfileCard.svelte`, `ParticipantMenu.svelte`,
new `components/NicknameDialog.svelte`, `FullProfile.svelte`.

- [ ] Tests: settings field saves `display_name` with preview, nudge shows while `display_name === username`, dismiss
  sticks; card shows `@username` small; "Change nickname" / "Reset nickname" / "Set private nickname…" call the right
  commands; a refused rename shows the server's words; full profile in a server lists that server's archive with
  "Renamed N times", Load more pages, resets shown as "(reset)".
- [ ] Implement, run, commit with `Closes #47`, `Closes #48`.
