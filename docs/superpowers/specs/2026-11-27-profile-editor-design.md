# Profile editor overhaul: design

Date: 2026-11-27. Target: one feature release on Monday 30 Nov, with the editor and the app-wide fixes together
(owner's call).

## Why

Profile editing lives in Settings → Profile (`client/ui/src/components/ProfileSettings.svelte`, about 830 lines). It
was built one field at a time into a column that was never meant for all of them, and it now feels cramped:

- The tab row (About you, Card, Name style, Speaking ring, Badges) scrolls sideways and cuts "Speaking ring" to
  "Spe…". It's hard to read and doesn't fit the rest of Settings.
- Sections are packed tight with a lot of explanatory text, and nothing flows.
- Avatar and banner are changed with text buttons ("Upload", "Remove"), even though the preview card already shows
  both.
- The Badges tab is a rigid numbered vertical list of small square tiles.
- **Card themes are broken:** the theme tiles render as empty pills. The `miniCard` snippet (banner, avatar and two
  lines) went in with #467 without any CSS (`.mini-card`, `.mc-*` have no styles anywhere).
- **Styled names get clipped across the app.** Taller fonts and effects lose their tops and bottoms on the profile
  card, in the channel list and on the stage (owner's screenshots).

The owner's direction: give profile editing its own large editor; use the preview itself as the editor wherever it
already shows the thing; less text, more pictures; put every set of ready-made options in its own popover so it doesn't
clutter the page. Principle: **if it's already displayed, edit it there; if it has a list of presets, open a popover.**

## Goals

1. A dedicated **profile editor**, a large modal, opened from everywhere that today opens Settings → Profile.
2. **The card is the editor** for what it shows: avatar and banner (pencil buttons on them), display name, pronouns
   and bio (click and type in place).
3. A **right-hand column of picture tiles** for the visual options, each tile a small preview of the current choice.
   Clicking a tile opens that option's popover.
4. **Popovers for preset-heavy options** (name style, card theme, speaking ring), each with its own live preview and
   Cancel / Apply.
5. A **badges section** rebuilt as a roomy grid with drag to arrange.
6. **Where you show up:** a small slice of the real app under the card showing you as others see you.
7. **Styled names never clip** anywhere in the app, and the card theme tiles show real previews.

## Non-goals

- New profile features or fields, except **Card stats** (below). Everything else in the editor exists today (or is
  App appearance, moved as is).
- Server or protocol changes, except the small additive ones Card stats needs. The save path (`profileDraft.ts`
  `toRequest`, `PATCH` profile) otherwise stays as it is.
- Redesigning App appearance itself. It moves into the editor's menu unchanged (see Layout). The wider "make Pulse
  yours" idea stays parked as #726.
- A board or widgets panel like the reference app's right side. Not wanted now.

## Anti-goals (a failure even if it works)

- Anything that spills out of its container, overlaps or jumps, at any window size (the standing UI rule).
- Explanatory text where a picture would do. Labels are short headings; tooltips only where a control is ambiguous.
- A styled name that clips anywhere: card, chat, channel list, stage, call band, member list, DM list, mentions.
- Losing unsaved edits silently.

## Approaches considered

Mocked up on the design canvas (private): four layouts plus two close-ups, then three variants of "Where you show up".

- **A · Studio:** categories on the left, the card centred, picture tiles on the right. Chosen as the base: the
  categories keep focus, and the centred card holds attention.
- **B · Card first:** one big card edited in place, a dock of tiles beneath. Too little room for badges and the
  other sections.
- **C · Tiles and preview:** grouped picture tiles on the left, card and "Where you show up" on the right. Its
  "Where you show up" idea was kept.
- **D · Focused pane:** today's structure with room to breathe. Moved too little.

For "Where you show up": a strip of four labelled boxes (felt off, because the boxes didn't look like Pulse and
didn't line up with the card), a column beside the card, a Card / Chat / Voice switch, and **a mini Pulse window
(chosen)**.

**Chosen: A, with C's "Where you show up" drawn as a mini Pulse window.**

## Layout (decided with the owner)

A modal, as large as the window allows: about 1280×880 at full size with an even margin, and shrinking with the
window. Three columns, a header and a footer:

```
┌ Edit profile ─────────────────────────────────────────────────────── [×] ┐
│ Looks      │            ┌──────── card ────────┐       │ Avatar & banner  │
│ Badges     │            │ banner          [✎]  │       │ [ avatar][banner]│
│ Typing text│            │ (avatar[✎])          │       │ ──────────────── │
│ ────────── │            │ Moony                │       │ Name             │
│ App        │            │ @moony · pronouns    │       │ [  Moony style  ]│
│ appearance │            │ badges               │       │ ──────────────── │
│            │            │ bio                  │       │ Card             │
│            │            │ stats                │       │ [theme] [glow ]  │
│            │            └──────────────────────┘       │ ──────────────── │
│            │   ┌ voice list ┬ chat ──────────────────┐ │ Speaking ring    │
│            │   │ Lounge     │ alex: new banner??     │ │ [style][colour]  │
│            │   │  (●)Moony  │ Moony: Looking good!   │ │                  │
│            │   │   alex     │ Moony is cooking…      │ │                  │
│            │   └────────────┴────────────────────────┘ │                  │
├──────────────────────────────────────────────────────── [Reset] [Save] ─┤
```

### Left: the menu

- **Looks** (default), **Badges**, **Card stats**, **Typing text**, then a divider, then **App appearance**.
- The menu replaces the tab row. Each entry changes the centre and right columns. It's a vertical list with icons,
  never scrolls sideways, and collapses to icons only when the window is narrow.
- App appearance is how Pulse looks to you, so it sits apart below the divider. It shows today's
  `AppearanceSettings` content in the centre and right columns, unchanged, and leaves Settings' menu. It saves
  instantly, as it does today, outside the profile Save. It must not depend on your profile loading: it opens and
  works even when the profile can't load (offline). The footer's Reset and Save are hidden on this page.

### Centre: the card, then "Where you show up"

- The live preview card (`ProfileCard` with `preview`), centred, about 370px wide.
- **Pencil buttons** on the banner (top right) and the avatar (bottom right) open a small menu: **Change** (the
  file picker, then the existing `CropDialog`) and **Remove** (only when one is set). Clicking the banner or avatar
  itself, or its right-hand tile, opens the same menu. While uploading, a spinner sits on the image itself. An upload
  error shows as a short line under the card. There are no text buttons for this anywhere.
- **Display name, pronouns and bio are edited on the card.** Click and type. The bio shows its raw text while
  focused, with an emoji button (today's `EmojiBox` picker) at the field's corner, and goes back to formatted
  (markdown, custom emoji) when you click away. The card must not jump when it switches: the field keeps its size. A soft dashed outline shows each field is
  editable on hover and focus, and a solid accent outline while typing. Limits and errors (`nameProblem`,
  `PRONOUNS_MAX`, `BIO_MAX`, line breaks) show as a short line under the field, on the card.
- **Where you show up**, under the card and about as wide as it (aligned, not full-width): a small slice of the real
  app with no labels or boxes. On the left, the voice channel list with you in a channel, your speaking ring lit. On
  the right, a short chat: someone's message, yours, and "‹you› is ‹typing text›…" at the bottom. Everything in it
  uses the draft (name style, avatar, ring, typing text) and the real components where practical (`Name`,
  `Avatar`, `SpeakingRing`), so a name that would clip shows clipped here first. The other person is a neutral test
  person (alex).

### Right: picture tiles (on Looks)

Grouped under short headings with a thin rule between groups:

- **Avatar & banner:** two tiles showing the current avatar and banner. Click: the same Change / Remove menu as the
  pencils.
- **Name:** one wide tile showing your name in its style. Click: the Name style popover.
- **Card:** Theme (a tile in the card's colours) and Glow (a tile showing the glow on or off). Theme opens the
  Card theme popover; Glow toggles.
- **Speaking ring:** Style (your ring on a sample avatar) and Colour (the chosen colour). Both open the Speaking ring
  popover.

On Badges and Typing text, the right column holds that section's controls (below). On App appearance, it holds
Appearance's.

### Popovers

They open over the editor, with the editor dimmed behind. Each one is large, with tiles on the left and a live preview
on the right (you on your card, in chat and in the channel list), plus Cancel and Apply. Apply writes into the editor's
draft. Nothing saves until the editor's Save.

- **Name style:** font (tiles showing "Gg" in each font), effect (tiles), colour (solid and gradient swatches, plus
  custom).
- **Card theme:** preset tiles drawn as miniature cards in their own colours (this fixes the empty pills), plus custom
  top and bottom colours.
- **Speaking ring:** "Still or looping" and "Follows your voice" tile grids, then the colour row. The preview runs the
  fake speech envelope that the current settings preview uses.

### Badges

The centre column keeps the card. The right column becomes a wider panel, or the section takes over the centre and
right columns together; that's the builder's call, as long as nothing is cramped.

- **On your card:** a horizontal row of five big slots (about 84px). Drag to reorder, drag out to remove, drop a badge
  in from the grid. An empty slot shows a dashed drop target.
- **Use my rarest five:** a toggle chip (today's `auto` showcase).
- **Your badges:** a grid of larger tiles (badge, name, how many people have it), with an accent outline on the ones
  already on your card. Click adds or removes; drag also works.
- Keyboard: the tiles are buttons, and the slots can be reordered with arrow keys (today's `railDrag` keyboard
  support carries over).

### Card stats (new, the owner's addition)

You choose which stats your card shows. Today the card's stats panel (`ProfileCard.svelte`) always shows Member since
and Voice, plus Steam completion and Steam (last 2 weeks) when you share Steam hours.

- The right column lists the four stats as rows, each with a show/hide switch and a drag handle to set their order.
  The card in the centre updates live.
- Stored on the server as part of the profile: a new nullable column on `users` (NULL means today's default, all
  shown in today's order), an additive optional field on the profile update request and on what the server sends.
- **The server applies it:** a hidden stat is left out of the profile it sends to other people, not just hidden by
  their client. You still see all of them in the editor so you can turn them back on.
- Steam's own sharing rules stay in charge. A Steam stat you've turned on still shows only while you share Steam
  hours. The live "In ‹voice channel›" line and the Deadlock panel are not stats and are unaffected.
- A migration with a rollback note in `deploy/README.md`. The server deploys before the client, as always.
- Build it last, so the editor never waits on it.

### Typing text

The typing text input with the suggestion chips, laid out with room. The "Where you show up" chat line previews it.

### Footer and closing

- **Reset** and **Save**, as today: Save is enabled only when the draft is dirty (`isDirty`).
- Closing with unsaved changes (×, Escape, clicking outside) asks once: keep editing or discard.

## App-wide: styled names never clip

`Name.svelte` and its uses get vertical room for the tallest font and effect. Rows that show a name (channel list,
member list, DM list, call band, stage tiles) may grow a few pixels to fit, but only as much as the tallest name style
needs (owner's call). Padding inside the clipping box is the expected method. The line box must fit ascenders,
descenders and effect overflow (text shadows, glows). Horizontal overflow ellipses inside the container. Check every
place a `Name` renders: the card, chat, channel list, stage tiles, call band, member list, DM list, mentions, the typing
line and popovers. Effects that animate (bounce, shimmer) must not move the layout around them. A test or Playwright
pass at a few widths covers the worst font and effect pairs.

## Entry points

- Every current route to Settings → Profile opens the editor instead: "Edit profile" on your own card
  (`profileUi.edit` → `Shell.svelte` `openSettings('profile')`), quick actions (`quickActions.ts` "Settings: Profile")
  and anything else calling `openSettings('profile')`.
- Settings' menu keeps a **Profile** entry that opens the editor (Settings closes first), so people who look there
  still find it. **Appearance** leaves Settings' menu; its quick action opens the editor on App appearance.

## Constraints

- Svelte 5, the existing components and the theme tokens in `app.css`. It must look right on every theme (dark,
  midnight, OLED, sunset, ember, plum, pastel, light and its tints).
- Reduced motion: no animated previews; still versions instead.
- The draft model (`profileDraft.ts`: `draftFrom`, `previewUser`, `previewCard`, `toRequest`, `isDirty`) is reused
  as is.
- The public repo rules: test people only (alex, sam, kim, jo).

## Left to the builder

- Component split: `ProfileSettings.svelte` breaks into an editor shell, the card-editing layer, the tile column,
  the three popovers, the Badges section and "Where you show up". Names and files are the builder's call.
- Exact sizes, spacing and the narrow-window behaviour, within the anti-goals.
- How inline editing on the card is done (`ProfileCard` gains an editing mode, or the editor overlays inputs on it).
- Whether the popovers reuse a shared shell.
- Tests: carry over what `ProfileSettings.test.ts` and `ProfileSettingsCss.test.ts` cover, mapped to the new pieces.

## Calls made in this doc (owner to check)

- **Activity & privacy,** which appeared in the mockups' menu, is dropped. Privacy stays its own Settings section.
- **Card stats** is show/hide plus drag to order (the owner asked for choosing what displays; ordering reuses the
  badge drag). If ordering isn't wanted, it's a cut, not a redesign.
- Clicking the **name text** on the card edits the display name in place. Its **style** is changed from the Name tile.
- Unsaved changes on close: one "keep editing or discard" prompt.
- App appearance works without the profile loaded, and hides Reset and Save.

## Answered after the builder check (owner)

- Name rows may grow a few pixels so styled names never clip.
- The bio is raw text while typing, with an emoji button, and formatted otherwise.
- The avatar and banner pencils (and tiles) open Change / Remove. Upload status shows on the image, errors under the card.
- Everything ships together on Monday.
