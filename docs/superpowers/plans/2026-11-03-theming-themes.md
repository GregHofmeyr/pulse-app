# Theming steps 2–4: themes, accent picker, readable names — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Theme switching (six themes, Light with a tint), an accent picker, and contrast-based readable names with a
per-device Name readability level, all in Settings → Appearance (#42 steps 2, 3, 4).

**Architecture:** Each theme is a CSS block in `app.css` selected by `data-theme` (and `data-tint` for Light) on
`<html>`; the blocks are attribute selectors, not `:root`-only, so a picker preview can carry its own `data-theme`. A
pure TS module (`lib/themes.ts`) knows each theme's surfaces, default accent and the colour maths (`readable`,
`deriveAccent`); tests pin it to the CSS. A small rune store (`lib/theme.svelte.ts`) persists the choices per device,
sets the attributes and the custom accent's derived tokens on `<html>`, and tells the Tauri core the window colour.

**Tech Stack:** Svelte 5 runes, vitest + jsdom, Tauri 2 (one small command).

**Spec:** `docs/superpowers/specs/2026-11-03-theming-design.md`, step-1 notes on #42, audit
`.superpowers/sdd/2026-11-03-theming-audit/audit.md` (gitignored).

## Global Constraints

- Every colour comes from a token (`colorTokens.test.ts`); new themes add values, not literals in components.
- Pulse Dark is today's look and the default; its token values don't change (shadows merge to levels, see Task 2).
- Light themes: raised things (`--bg-2`, `--surface-menu`) white with soft shadows on a slightly tinted ground; hover is
  a faint darkening; the tint touches surfaces and lines only, never text.
- No `color-mix()` / relative colour in `:root` values (old WebKitGTK); the accent family is set from JS as rgba().
- Accent: `--on-accent` is `#141519` or `#ffffff`, whichever has more contrast; `--accent-text` ≥ 4.5:1 on the theme's
  text surfaces (`--bg-1`, `--bg-2`, `--bg-3`, `--surface-menu`); `--focus-ring` ≥ 3:1 on the same.
- Name readability levels: Low 3:1 · Normal 4.5:1 (default) · High 7:1, measured against the surface the name sits on.
- Settings are per device (localStorage, try/catch like `lib/appearance.svelte.ts`).
- Server untouched; stored name colours unchanged; readability applied at render.
- Rust: only the window background. Cargo via the shared flock line; tests `--test-threads=4`.

## Review Focus

1. A corrupted / unknown saved value (theme `"ember"`, accent `"red"`, level `"9"`) → falls back to the default, never
   a broken page. Tested in Task 3.
2. A very light accent on Light (e.g. `#fffde0`) or a very dark one on OLED (`#050505`) → buttons still readable
   (`--on-accent`), links readable (`--accent-text`), focus ring visible (`--focus-ring`). Tested in Task 1 sweep.
3. A name colour already readable is left exactly as stored (no needless shift); gradients move per stop. Task 4.
4. The profile card is a dark island: a name there uses the dark rule even on Light. Task 4.
5. A preview inside the picker shows its own theme even when the app is on another one (every theme block declares
   every colour token, so nothing leaks from `<html>`). Task 2.

---

### Task 1: Colour maths and the theme table (`lib/themes.ts`)

**Files:** Create `client/ui/src/lib/themes.ts`, `client/ui/src/lib/themes.test.ts`.

**Interfaces (produces):**
- `type ThemeKey = 'dark' | 'midnight' | 'oled' | 'sunset' | 'pastel' | 'light'`; `type TintKey = 'plain' | 'pink' | 'orange' | 'blue' | 'green'`
- `type Surfaces = { bg0: string; bg1: string; bg2: string; bg3: string; menu: string }`
- `THEMES: { key; label; blurb; light: boolean; accent: string; surfaces: Surfaces }[]`; `TINTS: { key; label; surfaces: Pick<Surfaces,'bg0'|'bg1'|'bg3'> }[]`
- `surfacesFor(theme: ThemeKey, tint: TintKey): Surfaces`
- `readable(fg: string, bgs: string[], ratio: number): string` — `fg` mixed toward white (dark bgs) or black (light
  bgs) in 1% steps until `min(contrast(c, bg))` ≥ ratio; unchanged if already there; hue kept.
- `deriveAccent(hex: string, s: Surfaces, light: boolean): Record<'--accent'|'--accent-soft'|'--accent-faint'|'--focus-soft'|'--focus-ring'|'--on-accent'|'--accent-text', string>`
  (soft/faint/focus alphas: dark .15/.08/.45 = today's, light .14/.07/.55).
- `ACCENT_PRESETS: { label; color }[]` (Periwinkle `#8b9cff`, Mint `#3ddc97`, Coral `#ff7a6b`, Gold `#f2c14e`, Pink `#ff6fb5`, Sky `#4cb8ff`, Violet `#a77bff`)
- `NAME_LEVELS = [{key:'low',ratio:3},{key:'normal',ratio:4.5},{key:'high',ratio:7}]`

- [ ] Tests: `readable` leaves a passing colour alone, darkens on white, lightens on near-black, keeps hue
  (HSL hue within 2°) for saturated input; `deriveAccent` for every preset × every theme/tint and a sweep
  (`#ffffff #fffde0 #f0f0f0 #808080 #050505 #000000 #ff0000 #00ff00 #0000ff #ffff00`): `--on-accent` ≥ 4.5 or the
  better of the two, `--accent-text` ≥ 4.5 on every text surface, `--focus-ring` ≥ 3; Pulse Dark default accent
  derives exactly today's tokens (`rgba(139, 156, 255, .15)` etc., `--on-accent #141519`, `--accent-text #8b9cff`).
- [ ] Run → fail; implement; run → pass; commit `theming: colour maths and theme table (#42)`.

### Task 2: Theme blocks in `app.css`

**Files:** Modify `client/ui/src/app.css`, `colorTokens.test.ts`, `appCss.test.ts`; Create `themeCss.test.ts`;
modify components using the old shadow tokens.

- Restructure: `:root, [data-theme='dark'] { colour tokens }`, then `[data-theme='midnight']`, `'oled'`, `'sunset'`,
  `'pastel'`, `'light'` blocks (each declaring every colour token, plus `color-scheme`), then
  `[data-theme='light'][data-tint='pink'|'orange'|'blue'|'green']` (surfaces + lines only). Non-colour tokens stay in a
  plain `:root` rule. Pulse Dark values unchanged.
- Shadows → three levels: `--shadow-1` (`0 4px 14px rgba(0, 0, 0, .35)`: jump pill), `--shadow-2`
  (`0 8px 24px rgba(0, 0, 0, .4)`: tooltip, notices, emoji picker, toast, mini preview), `--shadow-3`
  (`0 18px 48px rgba(8, 9, 12, .5)`: menus, dialogs). Old names stay as aliases (`--shadow-menu: var(--shadow-3)` …)
  for branches in flight. Light: soft low-alpha shadows.
- `:focus-visible` outline uses `--focus-ring`.
- [ ] Tests (`themeCss.test.ts`): every theme block declares the same colour-token set as Pulse Dark; tint blocks only
  surface/line tokens; `themes.ts` surfaces equal the CSS values; contrast matrix — `--text`, `--text-body`, `--text-2`
  ≥ 4.5 on bg-0…bg-4, menu, inset; `--text-row`, `--text-3`, `--danger-text`, `--warn-text`, `--ok-text`,
  `--quote-green-text`, `--accent-text` ≥ 4.5 on bg-0, bg-1, bg-2, inset (new themes also bg-3 and menu) and ≥ 3 on
  bg-3/bg-4/menu (Pulse Dark today: 3.85–4.49 there, kept as the owner approved); `--on-danger` on `--danger` ≥ 4.5;
  the CSS accent family equals `deriveAccent(theme.accent)`; `color-scheme` light on light themes; island and tooltip
  tokens identical across themes.
- [ ] Update `colorTokens.test.ts` ("colour values in theme blocks only"; shadows pinned to levels).
- [ ] Run → fail; implement; run → pass; commit `theming: theme token blocks (#42)`.

### Task 3: Theme store (`lib/theme.svelte.ts`) and boot

**Files:** Create `lib/theme.svelte.ts`, `lib/theme.test.ts`; modify `main.ts`, `lib/tauri.ts`.

**Interfaces (produces):** `theme = $state({ key: ThemeKey, tint: TintKey, accent: string | null, names: 'low'|'normal'|'high' })`;
`setTheme(k)`, `setTint(t)`, `setAccent(hex | null)`, `setNameReadability(l)`; `applyTheme(root = document.documentElement)`;
`currentSurfaces(): Surfaces`; `nameRatio(): number`; `api.setWindowBackground(hex)`.
- Keys `pulse.theme`, `pulse.lightTint`, `pulse.accent`, `pulse.nameReadability`; bad values → defaults.
- `applyTheme` sets `data-theme`, `data-tint` (Light only), the custom accent family inline (removed when `null`), and
  calls `setWindowBackground(bg0)` only inside Tauri, errors swallowed.
- [ ] Tests: defaults; persistence round trip; corrupted values; throwing localStorage; attributes and inline
  properties on a root; custom accent cleared. Commit `theming: per-device theme store (#42)`.

### Task 4: Readable names

**Files:** Modify `lib/nameStyle.ts`, `lib/nameStyle.test.ts`, `components/Name.svelte`, `MessageItem.svelte`
(mention css), sidebar callers (`ChannelList`, `ConversationList`, `MemberList`, `GroupMembers`, `UserTile`,
`Shell` account bar) → `surface="side"`, `ProfileCard` → `surface="island"`.

**Interfaces:** `resolveNameStyle(user, { roleColor, showStyles, bg?: string, ratio?: number })` — with `bg`, every
colour (solid, role, each gradient stop) goes through `readable(c, [bg], ratio)` instead of `lightenForName`.
`Name` prop `surface?: 'chat' | 'side' | 'island'` (default `'chat'`); island bg `#262830` (the dark card).
- [ ] Tests: every preset solid/gradient at each level ≥ ratio on each theme's chat and side surface; unchanged when
  already readable; role colour follows; gradient per stop; island on Light still light; Name re-renders on theme
  change. Commit `theming: names readable by contrast, per-device level (#42)`.

### Task 5: Window background (Rust)

**Files:** Create `client/src-tauri/src/window_bg.rs`; modify `lib.rs`, `commands.rs` (or the new module) and the
handler list.
- `window-bg` file (`#rrggbb`) in the profile data dir; `load(dir) -> Option<Color>`, `save(dir, &str)`; startup uses
  it (falls back to `#141519`); command `set_window_background(color: String)` validates, sets it on the main window,
  saves.
- [ ] Unit tests for parse/load/save; fmt, clippy, test. Commit `theming: window background follows the theme (#42)`.

### Task 6: Settings → Appearance

**Files:** Modify `components/AppearanceSettings.svelte`, its test.
- Theme: a grid of cards, each a small live preview (`data-theme` on the card: sidebar strip, chat lines, an accent
  button) + label; `aria-pressed`. Light shows a tint row (5 swatches). Accent: "Theme default" + presets + an
  `<input type=color>`. Name readability: a 3-stop range (Low / Normal / High) with a live sample of four name styles
  on the chat surface. Existing "Show name styles" switch kept.
- [ ] Tests: picking a theme sets `data-theme` and saves; tint row only on Light; preset and custom accent set the
  inline tokens; slider changes the sample's colours and saves. Commit `theming: Appearance pickers (#42)`.

### Task 7: Visual pass

- Grep for dark assumptions (`--bg-1` cut-outs on other surfaces, `filter: brightness`, text on fixed dark boxes) and
  fix per theme with tokens; walk every view in the dev build per theme where it can run without a server.
- Commit `theming: visual pass fixes (#42)`.
