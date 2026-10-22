# Profile customisation: design

Date: 2026-10-22. Status: for the owner's review (builder check done, answers folded in).

Depends on: the **media store** from `2026-10-22-reactions-emoji-design.md` (§5). **This push is built after the
reactions and emoji push has merged** and uses its store as is (schema, serving, login-only fetch, cache, cleanup,
backups); it adds only the avatar and banner kinds. Related: badges (the trophy case slots in later), quotes (none;
the two can be built in either order).

## 1. Intent

Profiles are how the group shows off. The owner and his partner both take pride in profiles where everything fits one
theme: his orange and purple with touches of pink, hers blue, moody and starry. Discord does this well: lots of room
for atmosphere, but the text and info are never fighting the art. Pulse should give the same freedom with the same
restraint.

Today Pulse has no profile cards at all: no quick card, no full profile. Users have a username and an unused
`avatar_hash` column. This push builds the cards and the customisation together.

### Goals

- A profile card that can look like the owner's or his partner's Discord profiles: banner art, avatar, a two-colour
  card theme with a glowing edge, a styled name, pronouns and a bio.
- Name styles (font and colour) show **everywhere** the name appears, not only on the card.
- No combination of choices can make text unreadable.

### Non-goals (this push)

- **Profile decorations** (avatar frames, nameplate art, profile effects). Wanted later; it needs a source of art.
- Badges and the trophy case (the badges push; the full profile leaves the space for it).
- Nickname renaming and the Name Archive's data (the original design's "anyone can rename" feature, not built yet).
  The full profile shows the Archive once that feature exists.
- Game collections, music status, wishlists, server tags, roles on the profile card (Discord extras the owner doesn't
  want).

### Anti-goals

- Looking cheap: harsh saturated name colours, unreadable text on busy banners.
- A wall of motion: every avatar in a busy channel animating at once.

## 2. Decisions made with the owner

| Topic | Decision |
|---|---|
| Banner | Image or **animated** GIF across the top of the quick card and the full profile. It's the back layer; everything else sits on top and is never covered by it. Fades into the card colour at its bottom edge. |
| Avatar | Image or **animated** GIF, cropped to a circle. Overlaps the banner's bottom edge with a cut-out ring and the status dot, like Discord. |
| Card theme | **Two colours, top to bottom**, tinting the whole card as a gradient, with a soft glow on the card's border in the same colours. Presets plus a custom pair. Text and panels adjust automatically so it stays readable. |
| Name font | Picked from a curated list (8 in the mockup: elegant serif, rounded, pixel, script, techy…). |
| Name colour | **Soft solid colours or two-colour gradients**, from curated palettes that read well on dark, plus a custom option that's nudged light enough to read. Gradients are clearly visible but not loud (the owner asked for them slightly stronger than the first mockup; v2 values in §4.3). |
| Where the name style shows | **Everywhere:** profile cards, chat, member lists, DMs, mentions. |
| Role colours | **Like Discord:** in a server where you have a coloured role, your name **keeps your font but takes the role colour**. Your own colour shows everywhere else (DMs, groups, servers where you have no coloured role, your profile cards). |
| Profile text | **Pronouns** and a **bio**, plus what the original design promised: the **Name Archive**, **stats** (voice hours, times renamed, most-used reaction, member since) and, later, the **trophy case**. |
| Decorations | Later, when there's art. |

Approved mockups (git-ignored, on the owner's machine): `.superpowers/brainstorm/276729-1792672138/content/`
`profile-cards.html` (two themes, chat, member list with a role) and `profile-editor-v2.html` (the editor).

## 3. The cards

### 3.1 Quick profile card

Opens when you click a name or avatar anywhere (chat, member list, voice tiles, mentions). From the mockup:

- banner (fading into the card), avatar overlapping it with the status dot;
- the styled name; under it the username and pronouns;
- the badge row (empty until badges);
- the bio;
- a small panel: member since, voice hours, and "In <voice channel>" if they're in one (from the original design);
- "View full profile" and a quick message box (from the original design; it opens or sends to your DM with them).
- The card's theme: the two colours as a gradient behind everything, the border glowing in the same colours.

### 3.2 Full profile

A larger window with the same look:

- **Left:** the card, bigger, with all of §3.1.
- **Right:** panels in the style of Discord's "Board" (rounded cards on the theme's tinted background):
  - **Stats:** voice hours (from the existing `voice_sessions`, server voice only, as the original design says),
    member since, most-used reaction (once reactions exist), times renamed (once nicknames exist);
  - **Name Archive** (once nicknames exist);
  - the **trophy case** slot, filled by the badges push.
- **My call:** a section with no data yet is hidden, not shown empty.
- **Stats scope (my call, after the builder check):** totals across **all servers** (voice hours from server voice
  channels only, never DMs or group calls, as the original design requires); **member since** is the Pulse account's
  creation date. The same figures show wherever the card is opened, so the profile request doesn't take a server.

### 3.3 The name everywhere

- Chat messages, the member list, DMs and the conversation list, voice tiles, mentions and the @ autocomplete all draw
  the name with the person's font and colour.
- **Role rule (above):** in a server where the person has a coloured role, the colour comes from the role; the font
  stays. Pulse doesn't have roles built yet (the tables exist). **This push builds only the client side (my call,
  after the builder check):** one name-style resolver that takes an optional role colour, tested with fixture data.
  The roles push sends each member's top coloured role and feeds it in. No role queries or protocol fields here.
- Nicknames, when they exist, use the person's style too.
- **My call:** sizes stay as they are today; a font only changes the face. Fonts that run large or small are scaled
  per font so a name lines up with the others (the pixel font in the mockup is the extreme case).
- **My call:** a setting under Appearance, "Show name styles", on by default, for anyone who prefers plain names.

### 3.4 Motion

- **My call:** animated avatars and banners play on the quick card, the full profile and when you hover a
  name/avatar; in chat, the member list and voice tiles they show their first frame. Respects the OS reduced-motion
  setting (first frame everywhere).
- **How (my call, after the builder check):** an `<img>` can't pause an animated WebP, so the media store keeps a
  **still first-frame variant** for every animated file (added to the emoji design's store, §4.2 there, so emoji get
  it too). Lists use the still; hover and cards use the animated one.

## 4. The editor

### 4.1 Layout

Settings → Profile. Settings on the left, a **live preview** on the right (the quick card plus a chat line), updating
as you change anything. Save and Reset buttons and an "Unsaved changes" marker; leaving with unsaved changes asks
first.

**Everything is staged until Save (my call, after the builder check)**, uploads included: an uploaded avatar or banner
is processed and stored right away but isn't used by the profile until Save, which sends the whole profile (text,
style and media hashes) in one request. Reset or leaving discards the change; the store's cleanup removes uploads
nothing uses after a grace period (**my call:** 24 hours).

### 4.2 Avatar and banner

- Upload or remove. PNG, JPEG, WebP or GIF; animated allowed for both.
- A crop step: a circle for the avatar, the banner's shape for the banner.
- **My calls:**
  - Processed on the server like custom emoji (media store, §5 of the emoji design): avatar to 256×256, banner to
    960×384 (the card shows it at about 300×104 and the full profile larger; 2× for sharp screens). WebP, animated
    WebP for animations. Original not kept.
  - Upload limits: 8 MB before processing. Animated output capped (for example 200 frames, 2 MB for a banner, 1 MB
    for an avatar), refused with a clear message past that.
  - Stored on the server disk (the emoji design's per-kind locations: avatars and banners are "disk").
- The existing `avatar_hash` column holds the avatar's media hash; the banner gets its own.

### 4.3 Card theme

- Presets from the mockup: **Sunset** `#8a3cff → #ff8a2a`, **Night sky** `#22325e → #0d1430`, **Forest**
  `#1f7a5a → #0f2b3a`, **Berry** `#c2457a → #3a1f5e`, **Plain** `#3a3d46 → #16171b` (the default for everyone).
- **Custom:** two colour pickers, top and bottom.
- **Readability (my call, required):** the card's surface is the theme mixed heavily towards near-black (the mockup
  uses about 35% theme colour), and text uses fixed light colours on top; panels darken further. A custom pair is
  accepted as is, because this mixing keeps any pair readable. A contrast check in tests proves the worst case (pure
  white or pure yellow theme) still meets normal text contrast.

### 4.4 Name font

- **My call:** 8 fonts, bundled with the app (open licences, so they look the same everywhere and work offline). The
  mockup's set is a good start: an elegant serif (Cormorant Garamond), rounded (Fredoka), pixel (Press Start 2P),
  script (Pacifico), handwriting (Caveat), two grotesques (Space Grotesk, Bricolage Grotesque) and techy (Orbitron).
  The builder may swap any for a better fit, keeping the variety. Plus "Default" (the app font).
- Only the name uses the font; usernames, pronouns and bio stay in the app font.

### 4.5 Name colour

- **Solid:** default (the normal text colour), peach `#ffc4a3`, lilac `#d4c2ff`, mint `#a8e6cf`, sky `#a3d8ff`.
- **Gradients (v2, the owner's "slightly more noticeable"):** Sunset `#ffbe7a → #ff7fae`, Ice `#8fd0ff → #c9a8ff`,
  Mint `#a2f7c2 → #8fd2ff`, Peach `#ffe58f → #ff9d96`, Aurora `#f6b6ff → #9fb4ff`.
- **Custom:** one colour or two. **My call:** each custom colour is raised to a minimum lightness so it reads on the
  app's dark backgrounds (shown in the preview as it will appear, so nothing surprises).

### 4.6 Pronouns and bio

- Pronouns: free text, up to 40 characters.
- Bio: up to 190 characters, emoji (including custom emoji, once they exist) allowed, line breaks allowed (up to 4).

## 5. Data and events (for the builder)

- `users` gains: `banner_hash`, `theme_top`, `theme_bottom`, `name_font`, `name_color` (one or two colours),
  `pronouns`, `bio`. `avatar_hash` is reused. (Or a `profiles` table; builder's choice.)
- The old `images(hash, bytes)` table from 0001 is unused; the media store replaces it. **My call:** drop it in this
  push's migration.
- The profile's style fields travel with the user wherever users are sent (so chat and the member list can draw names
  without extra requests). Banner, bio and stats load when a card opens.
- Gateway: a profile-updated event to everyone who can see the user (the same audience as presence today), so names
  and avatars change live.
- The server validates everything: font from the list, colours valid, custom colours lightened server-side too, text
  limits, media kinds.

## 6. Tests

- Server: profile validation (fonts, colours, lengths), custom-colour lightening, media processing for avatars and
  banners (sizes, animation caps, wrong types), profile-updated events reach the right people.
- UI: the editor (presets, custom pickers, preview updates, unsaved-changes guard, crop), the quick card and full
  profile render with each preset theme, name styles in chat and the member list, the role-colour rule (with a test
  role), the "Show name styles" setting, reduced motion and first-frame display in lists, hidden empty sections.
- A contrast test for the theme mixing (§4.3) and the minimum name lightness (§4.5).
- Manual on Windows: the owner and his partner recreate their Discord profiles and compare.

## 7. Left to the builder

- Component structure, the crop tool, the exact mixing formula (within §4.3's rule), per-font size scaling.
- Whether the full profile is a modal or a page.
