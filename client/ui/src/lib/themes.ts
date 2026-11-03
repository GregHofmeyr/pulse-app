// The themes (#42, docs/superpowers/specs/2026-11-03-theming-design.md): what the app needs to know about each one
// outside CSS. The colours themselves live in app.css (one block per theme, switched by data-theme on <html>); this
// table repeats the surfaces names and the accent are measured against, and themeCss.test.ts keeps the two equal.
import { contrast, readable, rgba } from './color'

export type ThemeKey = 'dark' | 'midnight' | 'oled' | 'sunset' | 'pastel' | 'light'
export type TintKey = 'plain' | 'pink' | 'orange' | 'blue' | 'green'

/** The surfaces text sits on: the window (--bg-0), sidebar (--bg-1), chat (--bg-2), raised rows (--bg-3), menus. */
export type Surfaces = { bg0: string; bg1: string; bg2: string; bg3: string; menu: string }

export type Theme = { key: ThemeKey; label: string; blurb: string; light: boolean; accent: string; surfaces: Surfaces }

export const THEMES: Theme[] = [
  { key: 'dark', label: 'Pulse Dark', blurb: 'Layered greys, no true black.', light: false, accent: '#8b9cff', surfaces: { bg0: '#141519', bg1: '#1b1c21', bg2: '#212228', bg3: '#2a2c33', menu: '#2e3037' } },
  { key: 'midnight', label: 'Midnight', blurb: 'Deeper and bluer, for late sessions.', light: false, accent: '#8b9cff', surfaces: { bg0: '#0b0d14', bg1: '#11141e', bg2: '#151926', bg3: '#1d2233', menu: '#1d2233' } },
  { key: 'oled', label: 'OLED Black', blurb: 'True black window and sidebar.', light: false, accent: '#8b9cff', surfaces: { bg0: '#000000', bg1: '#000000', bg2: '#0c0c0e', bg3: '#1a1b1f', menu: '#161619' } },
  { key: 'sunset', label: 'Sunset', blurb: 'Warm rosy dark with a coral accent.', light: false, accent: '#ff8a6b', surfaces: { bg0: '#1c1213', bg1: '#23171a', bg2: '#2a1c1f', bg3: '#352427', menu: '#3a282b' } },
  { key: 'pastel', label: 'Pastel', blurb: 'Soft lavender, pink and peach.', light: true, accent: '#b48cf2', surfaces: { bg0: '#e8def3', bg1: '#f4ebf6', bg2: '#fffafb', bg3: '#f5e6ef', menu: '#fffafb' } },
  { key: 'light', label: 'Light', blurb: 'White panels on a softly tinted ground.', light: true, accent: '#4c5bd4', surfaces: { bg0: '#e9eaed', bg1: '#f4f4f6', bg2: '#ffffff', bg3: '#ececef', menu: '#ffffff' } },
]

export const DEFAULT_THEME: ThemeKey = 'dark'

/** Light's tints: the ground, sidebar and raised rows (and lines, in CSS) only; panels and menus stay white. */
export const TINTS: { key: TintKey; label: string; surfaces: Pick<Surfaces, 'bg0' | 'bg1' | 'bg3'> }[] = [
  { key: 'plain', label: 'Plain', surfaces: { bg0: '#e9eaed', bg1: '#f4f4f6', bg3: '#ececef' } },
  { key: 'pink', label: 'Pink', surfaces: { bg0: '#f0e6ea', bg1: '#f8f1f4', bg3: '#f2e8ec' } },
  { key: 'orange', label: 'Orange', surfaces: { bg0: '#f1e9e2', bg1: '#f9f4ef', bg3: '#f3ebe3' } },
  { key: 'blue', label: 'Blue', surfaces: { bg0: '#e5eaf2', bg1: '#f1f4f9', bg3: '#e8edf5' } },
  { key: 'green', label: 'Green', surfaces: { bg0: '#e5eee8', bg1: '#f1f6f2', bg3: '#e8f0ea' } },
]

export const DEFAULT_TINT: TintKey = 'plain'

export function themeByKey(key: string): Theme {
  return THEMES.find((t) => t.key === key) ?? THEMES[0]
}

export function surfacesFor(theme: ThemeKey, tint: TintKey): Surfaces {
  const t = themeByKey(theme)
  if (t.key !== 'light') return t.surfaces
  return { ...t.surfaces, ...(TINTS.find((n) => n.key === tint) ?? TINTS[0]).surfaces }
}

/** The surfaces accent text has to read on. */
export const textSurfaces = (s: Surfaces): string[] => [s.bg0, s.bg1, s.bg2, s.bg3, s.menu]

export const ACCENT_PRESETS: { label: string; color: string }[] = [
  { label: 'Periwinkle', color: '#8b9cff' },
  { label: 'Mint', color: '#3ddc97' },
  { label: 'Coral', color: '#ff7a6b' },
  { label: 'Gold', color: '#f2c14e' },
  { label: 'Pink', color: '#ff6fb5' },
  { label: 'Sky', color: '#4cb8ff' },
  { label: 'Violet', color: '#a77bff' },
]

/** The dark text on a light accent (today's --on-accent). */
export const DARK_INK = '#141519'

export type AccentTokens = Record<'--accent' | '--accent-soft' | '--accent-faint' | '--focus-soft' | '--focus-ring' | '--on-accent' | '--accent-text', string>

/**
 * Everything accent-coloured, from one colour: the translucent tints (mentions and chips, a mention's row, the message
 * box's focus edge), the focus ring (3:1 on the theme's surfaces), the text on an accent button (dark ink or white,
 * whichever reads better) and accent-coloured text (4.5:1 on the theme's surfaces).
 */
export function deriveAccent(hex: string, s: Surfaces, light: boolean): AccentTokens {
  // Dark ink or white, whichever reads better; a mid-tone where the ink falls under 4.5:1 gets pure black (black or
  // white always reaches 4.58:1).
  const better = contrast(DARK_INK, hex) >= contrast('#ffffff', hex) ? DARK_INK : '#ffffff'
  const on = contrast(better, hex) >= 4.5 ? better : '#000000'
  const bgs = textSurfaces(s)
  return {
    '--accent': hex,
    '--accent-soft': rgba(hex, light ? 0.14 : 0.15),
    '--accent-faint': rgba(hex, light ? 0.07 : 0.08),
    '--focus-soft': rgba(hex, light ? 0.55 : 0.45),
    '--focus-ring': readable(hex, bgs, 3),
    '--on-accent': on,
    '--accent-text': readable(hex, bgs, 4.5),
  }
}

export type NameLevel = 'low' | 'normal' | 'high'

/** Name readability: the contrast a name is moved to on the surface it sits on. */
export const NAME_LEVELS: { key: NameLevel; label: string; ratio: number }[] = [
  { key: 'low', label: 'Low', ratio: 3 },
  { key: 'normal', label: 'Normal', ratio: 4.5 },
  { key: 'high', label: 'High', ratio: 7 },
]

export const DEFAULT_NAME_LEVEL: NameLevel = 'normal'
