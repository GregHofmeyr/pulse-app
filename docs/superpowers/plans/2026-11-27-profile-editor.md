# Profile editor overhaul: implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace Settings → Profile with its own large profile editor: edit on the card, picture tiles, popovers
for presets, a draggable badge grid, a mini "Where you show up" window, Card stats, and styled names that never clip
anywhere.

**Architecture:**
- The draft, upload and save logic in `ProfileSettings.svelte` moves into one state module that the new editor
  shell and its sections share. The UI splits into focused components under `client/ui/src/components/profileEditor/`.
- Card stats is the only server change: one nullable column, an additive request field and an additive card field,
  with the server leaving hidden stats out of what others receive.

**Tech Stack:** Svelte 5 (runes), Vitest + Testing Library, Tauri 2 client; Rust axum + SQLite (sqlx) server;
`pulse-protocol` with ts-rs bindings (`just gen-types`).

**Spec:** `docs/superpowers/specs/2026-11-27-profile-editor-design.md` (read it with this plan). Tracks #727.

## Global Constraints

- Nothing spills out of its container, overlaps or jumps, at any window size (spec Anti-goals).
- Less text: short headings only, no explanatory paragraphs. Tooltips only where a control is ambiguous.
- Theme tokens from `client/ui/src/app.css` only. It must look right on dark, midnight, OLED, sunset, ember, plum,
  pastel, light and the light tints.
- Reduced motion (`motion.reducedMotion`): no animated previews, still versions instead.
- Draft model reused as is: `draftFrom`, `previewUser`, `previewCard`, `toRequest`, `isDirty` in
  `client/ui/src/lib/profileDraft.ts` (extended only for Card stats in Task 12).
- Nothing saves until **Save**, except App appearance, which saves instantly as today.
- Menu order: Looks, Badges, Card stats, Typing text, divider, App appearance.
- Test people only in fixtures and the mini window: alex, sam, kim, jo.
- Commits reference #727 and end with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. Never
  `git add -A`.
- Low-load builds: prefix cargo with
  `CARGO_BUILD_JOBS=2 CARGO_PROFILE_DEV_DEBUG=line-tables-only nice -n 19 ionice -c3`. Node is at
  `~/.config/nvm/versions/node/v22.22.3/bin`. Run targeted tests locally; CI runs the full suite.
- UI test command: `cd client/ui && pnpm exec vitest run <file>`. Type check: `cd client/ui && pnpm check`.

## Review Focus

1. **Escape with a popover open** closes only the popover (its Cancel), never the editor or its unsaved-changes
   prompt. Test in Task 4.
2. **Cancel in a popover restores the draft exactly,** including a custom colour picked mid-way (the `perFrame`
   pickers flush late). Test in Task 4.
3. **A narrow window (900×600)** keeps the editor inside the window with no horizontal scroll, and the menu goes to
   icons only. Test in Task 2 (CSS test on the breakpoint rules).
4. **The profile fails to load (offline):** Looks shows the existing error and retry, while App appearance still
   opens and works. Test in Task 2.
5. **A long display name, a 190-character bio and many line breaks** stay inside the card while editing and after.
   The name ellipses, the bio field scrolls inside its own box, and the card doesn't change width. Test in Task 3.

---

## File structure

New, under `client/ui/src/components/profileEditor/`:

| File | Responsibility |
|---|---|
| `ProfileEditor.svelte` | The modal: header, menu, the active section, footer, the unsaved-changes prompt |
| `EditorMenu.svelte` | The vertical menu, collapsing to icons when narrow |
| `CardEditor.svelte` | The live card with in-place name, pronouns and bio, and the avatar/banner pencil menus |
| `LooksTiles.svelte` | The right column of picture tiles on Looks |
| `EditorPopover.svelte` | Shared popover shell: tiles left, live preview right, Cancel / Apply, Escape = Cancel |
| `NameStylePopover.svelte`, `CardThemePopover.svelte`, `RingPopover.svelte` | The three popovers |
| `ShowUp.svelte` | "Where you show up": the mini Pulse window |
| `BadgesSection.svelte` | Slots row and badge grid with drag |
| `CardStatsSection.svelte` | Stat rows with show/hide and drag order |
| `TypingSection.svelte` | Typing text input and suggestion chips |
| `MiniCard.svelte` | A card in miniature in given colours (theme tiles) |

New `client/ui/src/lib/profileEditor.svelte.ts`: the open state, plus the `ProfileEditorState` class (draft, saved,
uploads, showcase, save, reset). Modified: `Name.svelte`, `ProfileCard.svelte`, `Settings.svelte`,
`routes/Shell.svelte`, `lib/quickActions.ts`. Deleted at the end: `ProfileSettings.svelte` (its tests are mapped to
the new files). Server: `server/migrations/0087_card_stats.sql`, `server/src/profiles/{mod.rs,routes.rs,users.rs}`,
`protocol/src/rest.rs`, `deploy/README.md`.

---

### Task 1: Styled names never clip (app-wide)

**Files:**
- Modify: `client/ui/src/components/Name.svelte` (the `<style>` block, `.name` and `.name[data-fx]` rules)
- Modify: the row CSS that pins name height, where a fixed height or `overflow: hidden` on a parent cuts the
  name: channel list voice rows, member list, DM list, call band, stage tiles (find them with
  `grep -rn "Name " client/ui/src/components/*.svelte`)
- Test: `client/ui/src/components/NameCss.test.ts` (new, same CSS-parsing style as `SpeakingRingCss.test.ts`)
- Test: `client/ui/e2e` or a Playwright check if the repo has one for UI fit; otherwise the CSS test only

**Interfaces:**
- Produces: the CSS custom property `--name-pad` (vertical room for the name, in em, set per font in
  `lib/nameFonts.ts` where a font needs more than the default). Later tasks render `Name` without extra clipping
  workarounds.

- [ ] **Step 1: Write the failing tests** in `NameCss.test.ts`:
  - `every_name_gets_vertical_room`: the `.name` rule has `padding-block: var(--name-pad, <n>em)` with `n >= 0.12`,
    and `overflow-clip-margin` applies to all names (not only `[data-fx]`).
  - `tall_fonts_get_more_room`: for each font in `NAME_FONTS` flagged `tall` in `lib/nameFonts.ts`
    (at least `pacifico`, `kaushan-script`, `caveat`, `cormorant-garamond`), its style sets `--name-pad >= 0.2em`.
  - `rows_holding_names_dont_pin_height`: none of the listed row selectors sets a fixed `height` together with
    `overflow: hidden` (use `min-height` instead).
- [ ] **Step 2: Run them and see them fail.** `pnpm exec vitest run src/components/NameCss.test.ts`, expecting
  FAIL on the missing padding.
- [ ] **Step 3: Implement.** In `Name.svelte`, add `padding-block` with matching `margin-block` of
  `calc(-1 * var(--name-pad))` only where a parent can't grow. Rows may grow a few px (owner's call), so prefer
  letting them grow. Add a `tall?: boolean` field and `--name-pad` to the font entries in `lib/nameFonts.ts`.
  Move `overflow-clip-margin: .35em` to `.name`. Swap fixed row heights for `min-height`.
- [ ] **Step 4: Run** the new test, `Name.test.ts` and the CSS tests of each touched component. Expect PASS. Run
  `pnpm check`.
- [ ] **Step 5: Commit** `fix(ui): styled names never clip (Refs #727)`.

### Task 2: Editor state, shell, menu, entry points and App appearance

**Files:**
- Create: `client/ui/src/lib/profileEditor.svelte.ts`
- Create: `components/profileEditor/ProfileEditor.svelte`, `EditorMenu.svelte`
- Modify: `components/Settings.svelte` (remove the `appearance` entry, make `profile` open the editor and close
  Settings), `routes/Shell.svelte` (`profileUi.edit` and `openSettings('profile')` open the editor),
  `lib/quickActions.ts` (Profile → editor on Looks, Appearance → editor on App appearance)
- Modify: `components/ProfileSettings.svelte` (only to consume `ProfileEditorState`, so it keeps working until Task 10)
- Test: `lib/profileEditor.test.ts`, `components/profileEditor/ProfileEditor.test.ts`,
  `components/profileEditor/ProfileEditorCss.test.ts`, update `Settings.test.ts`, `quickActions.test.ts`

**Interfaces:**
- Produces:
  - `export type EditorSection = 'looks' | 'badges' | 'stats' | 'typing' | 'appearance'`
  - `export const profileEditor: { open: boolean; section: EditorSection }` (`$state`)
  - `export function openProfileEditor(section?: EditorSection): void` (default `'looks'`)
  - `export function closeProfileEditor(): void`
  - `export class ProfileEditorState` with:
    - `draft: Draft` (`$state`), `saved: Draft`, `readonly dirty: boolean` (`isDirty`)
    - `showcase: string[] | 'auto'`, `uploading: Record<'avatar' | 'banner', boolean>`, `uploadError: string`
    - `saving: boolean`, `error: string`
    - `constructor(profile: Profile)`, `reset(): void`, `save(): Promise<void>`
    - `upload(kind: 'avatar' | 'banner', file: File, crop: CropRect): Promise<void>`, `remove(kind): void`
  - Lifted as is from `ProfileSettings.svelte` (`save` at ~382, `reset` ~370, `upload` ~245, the showcase at ~76).

- [ ] **Step 1: Write the failing tests.**
  - `profileEditor.test.ts`:
    - `open_defaults_to_looks`
    - `state_dirty_after_edit_and_clean_after_reset`
    - `save_sends_toRequest_of_the_draft` (mock `api`)
  - `ProfileEditor.test.ts`:
    - `menu_lists_sections_in_order`: Looks, Badges, Card stats, Typing text, App appearance, with a separator
      before App appearance.
    - `close_with_unsaved_changes_asks_once`: Keep editing stays open; Discard closes and resets.
    - `close_when_clean_closes_without_asking`
    - `appearance_works_when_profile_fails_to_load` (Review Focus 4): the profile load rejects; Looks shows the
      error with retry; App appearance renders `AppearanceSettings`.
    - `appearance_hides_reset_and_save`
  - `ProfileEditorCss.test.ts`: `narrow_window_menu_icons_only_and_no_horizontal_scroll` (Review Focus 3). Under
    the editor's narrow breakpoint (window width ≤ 960px), menu labels are hidden, the modal has
    `max-width: calc(100vw - 2 * margin)`, and no fixed widths add up beyond it.
  - `Settings.test.ts`:
    - `no_appearance_entry`
    - `profile_entry_opens_editor_and_closes_settings`
- [ ] **Step 2: Run them and see them fail.**
- [ ] **Step 3: Implement.**
  - The modal is about 1280×880 at most, with an even margin, shrinking with the window.
  - The menu entries have icons. App appearance mounts `AppearanceSettings` unchanged, outside the profile load gate.
  - Escape and a click outside go through the same close guard.
  - Looks shows a plain `ProfileCard preview` for now; Task 3 replaces it.
- [ ] **Step 4: Run** the tests above plus `ProfileSettings.test.ts` (still green on the shared state). Run
  `pnpm check`.
- [ ] **Step 5: Commit** `feat(profile): the profile editor's shell, menu and entry points (Refs #727)`.

### Task 3: Edit on the card

**Files:**
- Create: `components/profileEditor/CardEditor.svelte`
- Modify: `components/ProfileCard.svelte`: add an `editable` mode with slots for the name, pronouns and bio
- Test: `components/profileEditor/CardEditor.test.ts`, extend `ProfileCard.test.ts` and `ProfileCardCss.test.ts`

**Interfaces:**
- Consumes: `ProfileEditorState` (Task 2).
- Produces: `CardEditor` props `{ state: ProfileEditorState }`. `ProfileCard` gains
  `editable?: { name: Snippet; pronouns: Snippet; bio: Snippet }`; when set, those render in place of the read-only
  text. `onEditAvatar` and `onEditBanner` stay.

- [ ] **Step 1: Write the failing tests.**
  - `pencils_open_change_remove_menu`: the menu has Change and Remove, and Remove shows only when an image is set.
  - `change_opens_file_picker_then_crop`
  - `remove_clears_the_draft_hash`
  - `upload_shows_spinner_on_the_image_and_error_under_the_card`
  - `name_pronouns_bio_edit_in_place`: typing updates `state.draft`.
  - `name_problem_and_limits_show_under_the_field`: `nameProblem`, `PRONOUNS_MAX`, `BIO_MAX`,
    `BIO_LINE_BREAKS_MAX`.
  - `bio_raw_while_focused_formatted_on_blur`: focused shows the raw text and an emoji button (`EmojiBox`); blurred
    renders markdown.
  - `long_inputs_stay_inside_the_card` (Review Focus 5): a long name, a 190-character bio and 8 line breaks. The
    card width is unchanged, the bio box scrolls, and the name has `text-overflow: ellipsis`.
  - CSS: `bio_field_keeps_size_between_raw_and_formatted`, using the same fixed min/max height rules in both states.
- [ ] **Step 2: Run them and see them fail.**
- [ ] **Step 3: Implement.**
  - A dashed outline shows on hover and focus, and a solid accent outline while typing.
  - The pencil buttons sit top right on the banner and bottom right on the avatar. Clicking the image itself opens
    the same menu.
  - Reuse `CropDialog` and `EmojiBox`.
- [ ] **Step 4: Run** the tests. Run `pnpm check`.
- [ ] **Step 5: Commit** `feat(profile): edit your name, pronouns, bio and pictures on the card (Refs #727)`.

### Task 4: Looks tiles, the popover shell and the Card theme popover

**Files:**
- Create: `components/profileEditor/LooksTiles.svelte`, `EditorPopover.svelte`, `CardThemePopover.svelte`,
  `MiniCard.svelte`
- Test: `LooksTiles.test.ts`, `EditorPopover.test.ts`, `CardThemePopover.test.ts`, `MiniCardCss.test.ts`

**Interfaces:**
- Consumes: `ProfileEditorState`, `THEME_PRESETS`, `cardVars`, `presetFor` from `lib/cardTheme.ts`.
- Produces:
  - `EditorPopover` props: `{ title: string; onCancel: () => void; onApply: () => void; tiles: Snippet;
    preview: Snippet }`. Escape calls `onCancel`. The editor behind is dimmed and inert.
  - Each popover takes `{ state: ProfileEditorState; onClose: () => void }`, copies the draft fields it edits on
    open, and on Cancel writes the copy back.
  - `MiniCard` props: `{ top: string; bottom: string }`.

- [ ] **Step 1: Write the failing tests.**
  - `tiles_grouped_under_headings`: Avatar & banner, Name, Card, Speaking ring.
  - `avatar_banner_tiles_open_change_remove_menu`
  - `glow_tile_toggles_draft_glow`
  - `theme_tile_opens_card_theme_popover`
  - `escape_in_popover_cancels_only_the_popover` (Review Focus 1): the editor stays open with no close prompt.
  - `cancel_restores_draft_exactly` (Review Focus 2): pick a custom top colour through the picker, then Cancel. The
    draft equals its pre-open value after the `perFrame` pickers flush.
  - `apply_keeps_changes_in_draft_unsaved`: Save is enabled and nothing has been sent.
  - `MiniCardCss`: `mini_card_draws_banner_avatar_and_lines` (the `.mc-banner`, `.mc-avatar`, `.mc-line` rules exist
    with sizes and colours from the theme vars). This fixes the empty theme pills.
- [ ] **Step 2: Run them and see them fail.**
- [ ] **Step 3: Implement.**
  - The tile faces show the current choice: the avatar, the banner, your name in its style, the theme as a
    `MiniCard`, the glow on or off, your ring on a sample avatar, and the ring colour.
  - The Card theme popover has preset tiles as `MiniCard`s plus custom top and bottom colour pickers, with the live
    preview on the right.
- [ ] **Step 4: Run** the tests. Run `pnpm check`.
- [ ] **Step 5: Commit** `feat(profile): picture tiles, the popover shell and card themes with real previews (Refs #727)`.

### Task 5: Name style popover

**Files:**
- Create: `components/profileEditor/NameStylePopover.svelte`
- Test: `NameStylePopover.test.ts`

**Interfaces:**
- Consumes: `EditorPopover` (Task 4), `NAME_FONTS`, `NAME_EFFECTS`, `NAME_SOLIDS`, `NAME_GRADIENTS`,
  `lightenForName`.
- Produces: `NameStylePopover` `{ state: ProfileEditorState; onClose: () => void }`, opened by the Name tile.

- [ ] **Step 1: Write the failing tests.**
  - `font_tiles_show_Gg_in_each_font`
  - `effect_tiles_cover_none_and_every_effect`
  - `colour_has_solids_gradients_and_custom`
  - `preview_shows_card_chat_and_list_name`
  - `cancel_restores_font_colour_effect`
- [ ] **Step 2: Run them and see them fail.**
- [ ] **Step 3: Implement**, lifting the font, colour and effect controls from `ProfileSettings.svelte` (the Name
  style tab). Use the tile layout instead of chips.
- [ ] **Step 4: Run** the tests.
- [ ] **Step 5: Commit** `feat(profile): the name style popover (Refs #727)`.

### Task 6: Speaking ring popover

**Files:**
- Create: `components/profileEditor/RingPopover.svelte`
- Test: `RingPopover.test.ts`

**Interfaces:**
- Consumes: `RING_STYLES`, `RING_COLORS`, `envelopeStep`, `isReactive`, `PREVIEW_D`, `SpeakingRing`.
- Produces: `RingPopover` `{ state: ProfileEditorState; onClose: () => void }`, opened by both ring tiles.

- [ ] **Step 1: Write the failing tests.**
  - `two_groups_still_or_looping_and_follows_your_voice`
  - `colour_row_includes_profile_colour`
  - `preview_runs_envelope_only_when_reactive_and_motion_allowed`
  - `cancel_restores_style_and_colour`
- [ ] **Step 2: Run them and see them fail.**
- [ ] **Step 3: Implement**, lifting the ring tab and its envelope preview from `ProfileSettings.svelte` (around
  lines 148–170 and 616–660), as tiles.
- [ ] **Step 4: Run** the tests.
- [ ] **Step 5: Commit** `feat(profile): the speaking ring popover (Refs #727)`.

### Task 7: Where you show up

**Files:**
- Create: `components/profileEditor/ShowUp.svelte`
- Test: `ShowUp.test.ts`, `ShowUpCss.test.ts`

**Interfaces:**
- Consumes: `previewUser(me, draft)`, `Name`, `Avatar`, `SpeakingRing` (static), `TYPING_SUGGESTIONS` / the
  typing line helper.
- Produces: `ShowUp` `{ state: ProfileEditorState }`, under the card on Looks.

- [ ] **Step 1: Write the failing tests.**
  - `voice_list_shows_you_lit_with_your_ring`
  - `chat_shows_alex_then_you`
  - `typing_line_uses_draft_typing_text`
  - `uses_draft_name_style_and_avatar`
  - CSS: `width_matches_the_card_and_no_labels_or_boxes`. Its width is tied to the card width variable, and it
    contains no heading elements.
- [ ] **Step 2: Run them and see them fail.**
- [ ] **Step 3: Implement** a left voice-list half (channel "Lounge", you with your ring lit, alex) and a right chat
  half (alex: "new banner??", you: "Looking good!", the typing line).
- [ ] **Step 4: Run** the tests.
- [ ] **Step 5: Commit** `feat(profile): where you show up, as a mini Pulse window (Refs #727)`.

### Task 8: Badges section

**Files:**
- Create: `components/profileEditor/BadgesSection.svelte`
- Test: `BadgesSection.test.ts`

**Interfaces:**
- Consumes: `railDrag` (`item: 'badge'`, `onDrop(id, index)`), `SHOWCASE_MAX`, `defaultShowcase`, `badgeStore`,
  `BadgeIcon`, `state.showcase`.

- [ ] **Step 1: Write the failing tests.**
  - `five_slots_in_a_row_with_dashed_empty_slots`
  - `click_in_grid_adds_or_removes`
  - `drag_reorders_slots` (`onDrop`)
  - `drag_out_removes`
  - `rarest_five_chip_sets_auto`
  - `grid_tile_shows_name_and_holder_count_and_outline_when_on_card`
  - `arrow_keys_reorder` (the `railDrag` keyboard support)
- [ ] **Step 2: Run them and see them fail.**
- [ ] **Step 3: Implement.** Slots are about 84px. The builder chooses between a wider right panel and a section
  spanning the centre and right columns, provided nothing is cramped.
- [ ] **Step 4: Run** the tests.
- [ ] **Step 5: Commit** `feat(profile): a badge grid you drag to arrange (Refs #727)`.

### Task 9: Typing text section

**Files:**
- Create: `components/profileEditor/TypingSection.svelte`
- Test: `TypingSection.test.ts`

- [ ] **Step 1: Write the failing tests.**
  - `input_and_suggestion_chips_set_draft`
  - `typing_problem_shows_under_input` (`typingTextProblem`)
  - `show_up_typing_line_follows`
- [ ] **Step 2: Run them and see them fail.**
- [ ] **Step 3: Implement**, lifting the typing text block from `ProfileSettings.svelte`.
- [ ] **Step 4: Run** the tests.
- [ ] **Step 5: Commit** `feat(profile): typing text section (Refs #727)`.

### Task 10: Retire ProfileSettings

**Files:**
- Delete: `components/ProfileSettings.svelte`, `ProfileSettings.test.ts`, `ProfileSettingsCss.test.ts`
- Modify: any import of them (`Settings.svelte`)

- [ ] **Step 1: Map each test** in `ProfileSettings.test.ts` and `ProfileSettingsCss.test.ts` to its new home
  (Tasks 2–9). Add any case not yet covered to the owning new test file, and list the mapping in the commit body.
- [ ] **Step 2: Delete the files**, then run the whole UI suite: `pnpm exec vitest run`. Expect PASS. Run
  `pnpm check`.
- [ ] **Step 3: Commit** `refactor(profile): retire the old Settings → Profile page (Refs #727)`.

### Task 11: Card stats, server

**Files:**
- Create: `server/migrations/0087_card_stats.sql`
- Modify: `protocol/src/rest.rs` (`CardProfile`, `UpdateProfileRequest`, `ProfileStats`),
  `server/src/profiles/mod.rs` (`load_profile`, the update parsing near `ring_key`),
  `server/src/profiles/routes.rs` (the `UPDATE users` statement), `server/src/blocks.rs` (the emptied profile),
  `deploy/README.md`
- Test: `server/tests/profiles.rs`

**Interfaces:**
- Produces:
  - `pub const CARD_STATS: [&str; 4] = ["member_since", "voice", "steam_completion", "steam_recent"]` in
    `server/src/profiles/mod.rs`, with a literal copy in the client test.
  - `CardProfile.stats: Option<Vec<String>>`: shown keys in order; `None` means the default (all, in that order).
    `#[serde(default, skip_serializing_if = "Option::is_none")]`, `#[ts(optional)]`.
  - `UpdateProfileRequest.card_stats: Option<Vec<String>>`: absent keeps the current setting. A list equal to the
    default stores NULL. Unknown or duplicate keys are rejected with "Unknown card stat." or "A card stat can be
    listed once.".
  - `ProfileStats.voice_seconds: Option<u64>` and `member_since: Option<String>`, both
    `#[serde(default, skip_serializing_if = "Option::is_none")]`. The server leaves out hidden ones when
    `viewer != user`. Steam fields become `None` when hidden.
  - Migration: `ALTER TABLE users ADD COLUMN card_stats TEXT;` (a JSON array, NULL means the default).
- Then run `just gen-types`.

- [ ] **Step 1: Write the failing tests** in `server/tests/profiles.rs`:
  - `card_stats_default_is_null_and_all_shown`
  - `save_card_stats_order_and_subset`
  - `default_list_stores_null`
  - `unknown_or_duplicate_key_rejected`
  - `hidden_stats_left_out_for_others`: alex hides voice and member_since; sam's view has neither, and alex's own
    view has both.
  - `steam_stat_shown_only_while_shared_even_if_on`
  - `absent_field_keeps_setting`
- [ ] **Step 2: Run them and see them fail:** `<low-load prefix> cargo test -p pulse-server --test profiles card_stats`.
- [ ] **Step 3: Implement**, with the rollback note in `deploy/README.md`: `ALTER TABLE users DROP COLUMN card_stats;`.
- [ ] **Step 4: Run** the profiles tests, `cargo test -p pulse-protocol`, `just gen-types` (commit the regenerated
  bindings) and `cargo clippy -p pulse-server --all-targets -- -D warnings`.
- [ ] **Step 5: Commit** `feat(server): choose which stats your card shows (Refs #727)`.

### Task 12: Card stats, client

**Files:**
- Create: `components/profileEditor/CardStatsSection.svelte`
- Modify: `lib/profileDraft.ts` (`Draft.cardStats: string[] | null`, `draftFrom`, `toRequest`, `previewCard`,
  `isDirty`), `components/ProfileCard.svelte` (render stats in `card.stats` order, skip absent values)
- Test: `CardStatsSection.test.ts`, `lib/profileDraft.test.ts`, `ProfileCard.test.ts`

**Interfaces:**
- Consumes: Task 11's fields and `CARD_STATS` order (the literal copy in `lib/cardStats.ts`, checked against the
  server list by a test).

- [ ] **Step 1: Write the failing tests.**
  - `rows_with_switch_and_drag_handle`
  - `toggle_and_reorder_update_card_live`
  - `toRequest_sends_card_stats_only_when_changed`
  - `profile_card_respects_order_and_skips_missing`
  - `old_server_without_stats_field_shows_today_default`
- [ ] **Step 2: Run them and see them fail.**
- [ ] **Step 3: Implement** with `railDrag` (`item: 'stat'`).
- [ ] **Step 4: Run** the tests, the whole UI suite and `pnpm check`.
- [ ] **Step 5: Commit** `feat(profile): card stats section (Refs #727)`.

---

## Spec gaps settled here

- **Hidden voice and member-since stats:** these fields become optional in `ProfileStats`, so the server can truly
  leave them out. A 0.2.20 client viewing someone who hid one may show a blank value until it updates. That's
  accepted, because the server ships first and the clients follow the same week.
- **Card stats keys and storage:** a JSON array in `users.card_stats`, keyed as in `CARD_STATS`.
- **Name clipping method:** vertical padding inside the name box, plus `overflow-clip-margin` on every name. Tall
  fonts are flagged in `lib/nameFonts.ts`, and rows switch from fixed heights to `min-height`.
