# Theming

Issue #42. Built on the colour audit (gitignored ledger `.superpowers/sdd/2026-11-03-theming-audit/audit.md`, plan on
#42) and the owner's verdicts in the theme lab (2026-11-03). **Status: approved by the owner 2026-11-03.**

## Why

People want Pulse to look like theirs: lighter, darker, warmer, their colour. Today the app has one dark look.

## Goals

- A theme picker in Settings → Appearance, saved per device.
- Themes: **Pulse Dark** (today, default), **Midnight**, **OLED Black**, **Light** (with a tint), a **Pastel** theme
  (purples, pinks, oranges), and a reworked warm theme to replace the lab's "Ember".
- An **accent colour picker**: presets plus any colour; everything accent-coloured follows, and text on accent stays
  readable.
- **Names stay readable on every theme**, with a per-user **Name readability** slider.
- More **name fonts** (each added the same way as today's eight).

## Non-goals

- User-made themes (editing every colour). Maybe later.
- Per-server themes.

## Owner's verdicts

- Dark, Midnight, OLED Black, the accent picker: love.
- **Light:** not stark white. A *very slight* tint, chosen by the user: plain, a slight pink, a slight orange (and maybe
  blue/green).
- **Ember:** warm idea good, colour off: rework it (warmer, less brown; builder proposes 2–3 options for the owner).
- **Pastel:** new: soft purples, pinks and oranges, colourful but calm.
- **Readable names:** make the strength adjustable per user, because screens differ.

## Rules

- A theme is a set of colour tokens (`app.css` `:root` today) switched by `data-theme` on the root. Every colour in the
  app comes from a token (the audit's S cleanup first: ~15 stray literals become tokens, the accent copies are derived,
  warn/ok/quote-green get text vs fill tokens).
- **Light themes don't use "higher = lighter".** Raised things (menus, cards, dialogs) are white with a soft shadow on a
  slightly tinted ground; hover is a faint darkening. The tint is applied to the neutral ladder only (surfaces and
  lines), never to text contrast.
- **Accent:** one chosen colour; the app derives the soft tint, the focus edge, the text colour on accent buttons (dark
  or white, whichever reads better), and an accent *text* colour that stays readable on the theme's surfaces (4.5:1).
- **Names:** names keep their hue; on each theme they're moved toward the readable side just enough to reach a contrast
  the user picks with **Name readability** (Low 3:1 · Normal 4.5:1 · High 7:1; Normal default). On dark themes today's
  rule (never darker than luminance 0.39) becomes the same contrast rule, so it adapts to Midnight and OLED too. Role
  colours and mention pills follow the same function.
- **Fixed on purpose:** video chrome, the QR code, skin-tone swatches, link-embed brand colours, and the Quote of the Year card (gets its own island tokens so it reads on every theme).
- **Profile cards follow the theme** (#139, owner testing 2026-11-03): they keep their own two colours, but on a
  light theme they're drawn light (the colours mixed into white, dark text) and their edge is moved just far enough
  to stand off the app's surfaces, so pale card colours still show. Dark themes keep the dark island.
- The Tauri window background follows the theme (no dark flash when starting on Light).

## How it gets made (builder's call where not stated)

1. **S — tokens only, no visual change:** the audit's cleanup list; tests pin today's values.
2. **M — theme switching:** `data-theme` blocks, `color-scheme` per theme, the per-device setting
   (`lib/appearance.svelte.ts` pattern), the picker with live previews, the window background, the light ladder; a
   visual pass on every view (sidebar, chat, panels, menus, dialogs, settings, clips, quotes, streams, search, embeds).
3. **M — accent picker:** the derive function + presets + custom colour; tests for contrast on every theme.
4. **M — readable names:** one contrast function used by names, role colours and mentions; the slider.
5. **S per font:** each new name font (client `lib/nameFonts.ts` + `nameFonts.css`, server `NAME_FONTS`, tests).

## Testing

- Contrast tests: every text token on every surface per theme ≥ 4.5:1 (3:1 for large/secondary where agreed); accent
  text and on-accent for every preset and a sweep of custom colours; names at each readability level.
- Snapshot-ish CSS tests per theme (as the app already pins tokens).
- Manual (owner): switch themes live; Light with each tint; pick odd accents (very light, very dark); names at each
  readability level on Light and OLED; a quick look at every view.
