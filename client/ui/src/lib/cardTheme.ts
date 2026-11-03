// Card themes (plan reconciliation 8): two raw colours, mixed into near-black so card text always reads. On a light
// app theme (#139) the card follows it: the colours mixed into white, dark text, and an edge that stands off the app.
import { luminance, readable } from './color'
import { parseHex, toHex } from './nameStyle'

export type ThemeKey = 'sunset' | 'night' | 'forest' | 'berry' | 'plain'
export type ThemePreset = { key: ThemeKey; label: string; top: string; bottom: string }

export const THEME_PRESETS: ThemePreset[] = [
  { key: 'sunset', label: 'Sunset', top: '#8a3cff', bottom: '#ff8a2a' },
  { key: 'night', label: 'Night sky', top: '#22325e', bottom: '#0d1430' },
  { key: 'forest', label: 'Forest', top: '#1f7a5a', bottom: '#0f2b3a' },
  { key: 'berry', label: 'Berry', top: '#c2457a', bottom: '#3a1f5e' },
  { key: 'plain', label: 'Plain', top: '#3a3d46', bottom: '#16171b' },
]

export const DEFAULT_THEME = THEME_PRESETS[4]

export const CARD_MIX = 0.3
export const PANEL_MIX = 0.18
export const NEAR_BLACK = '#0d0e12'
export const CARD_TEXT = '#f2f3f6'
export const CARD_TEXT_2 = '#d6d8df'

// The light card (#139): more colour than the dark one (white washes it out faster), panels a little whiter.
export const CARD_MIX_LIGHT = 0.4
export const PANEL_MIX_LIGHT = 0.22
export const NEAR_WHITE = '#ffffff'
export const CARD_TEXT_LIGHT = '#17181c'
export const CARD_TEXT_2_LIGHT = '#464953'
/** The least contrast a light card's edge has on the app's surfaces: a white or pale colour turns grey enough to see. */
export const EDGE_RATIO_LIGHT = 1.5
// The voice-room line and errors on a light card start from these and darken to 4.5:1 on its surfaces (the theme's own
// -text tokens are tuned for near-white, and a card is tinted).
export const CARD_OK_LIGHT = '#1a7f4b'
export const CARD_DANGER_LIGHT = '#c0262f'

/** The app theme a card sits in: light or dark, and (light) the app surfaces its edge must stand off. */
export type CardLook = { light: boolean; ground: string[] }
export const DARK_LOOK: CardLook = { light: false, ground: [] }

// Each raw edge colour at 45% alpha is its glow (reconciliation 8).
const GLOW_ALPHA = '73'

export function presetFor(top: string, bottom: string): ThemeKey | null {
  const t = top.toLowerCase()
  const b = bottom.toLowerCase()
  return THEME_PRESETS.find((p) => p.top === t && p.bottom === b)?.key ?? null
}

/** `color` mixed `weight` (0..1) into `base`. */
export function mixHex(color: string, base: string, weight: number): string {
  const c = parseHex(color) ?? parseHex(NEAR_BLACK)!
  const b = parseHex(base) ?? parseHex(NEAR_BLACK)!
  return toHex(c.map((v, i) => Math.round(v * weight + b[i] * (1 - weight))) as [number, number, number])
}

/** The CSS custom properties a card (or its preview, or the full profile) sets on its root. */
export function cardVars(top: string, bottom: string, look: CardLook = DARK_LOOK): Record<string, string> {
  if (!look.light) {
    return {
      '--card-top': mixHex(top, NEAR_BLACK, CARD_MIX),
      '--card-bottom': mixHex(bottom, NEAR_BLACK, CARD_MIX),
      '--panel-top': mixHex(top, NEAR_BLACK, PANEL_MIX),
      '--panel-bottom': mixHex(bottom, NEAR_BLACK, PANEL_MIX),
      '--glow-top': top,
      '--glow-bottom': bottom,
      '--glow-top-a': `${top}${GLOW_ALPHA}`,
      '--glow-bottom-a': `${bottom}${GLOW_ALPHA}`,
      '--card-text': CARD_TEXT,
      '--card-text-2': CARD_TEXT_2,
      // the island's own greys and colours, the same in every dark theme
      '--card-line': 'rgb(242 243 246 / .08)',
      '--card-hover': 'rgb(242 243 246 / .08)',
      '--card-skeleton': 'rgb(242 243 246 / .12)',
      '--card-edge': 'rgb(242 243 246 / .14)',
      '--card-danger': '#ff9d96',
      '--card-ok': 'var(--ok)',
      '--card-lift': '0 0 #0000',
    }
  }
  const surfaces = {
    '--card-top': mixHex(top, NEAR_WHITE, CARD_MIX_LIGHT),
    '--card-bottom': mixHex(bottom, NEAR_WHITE, CARD_MIX_LIGHT),
    '--panel-top': mixHex(top, NEAR_WHITE, PANEL_MIX_LIGHT),
    '--panel-bottom': mixHex(bottom, NEAR_WHITE, PANEL_MIX_LIGHT),
  }
  const on = Object.values(surfaces)
  const edge = (c: string) => (look.ground.length && parseHex(c) ? readable(c.toLowerCase(), look.ground, EDGE_RATIO_LIGHT) : c)
  const [glowTop, glowBottom] = [edge(top), edge(bottom)]
  return {
    ...surfaces,
    '--glow-top': glowTop,
    '--glow-bottom': glowBottom,
    '--glow-top-a': `${glowTop}${GLOW_ALPHA}`,
    '--glow-bottom-a': `${glowBottom}${GLOW_ALPHA}`,
    '--card-text': readable(CARD_TEXT_LIGHT, on, 7),
    '--card-text-2': readable(CARD_TEXT_2_LIGHT, on, 4.5),
    '--card-line': 'rgb(23 24 28 / .1)',
    '--card-hover': 'rgb(23 24 28 / .06)',
    '--card-skeleton': 'rgb(23 24 28 / .1)',
    '--card-edge': 'rgb(23 24 28 / .16)',
    '--card-danger': readable(CARD_DANGER_LIGHT, on, 4.5),
    '--card-ok': readable(CARD_OK_LIGHT, on, 4.5),
    // raised things on a light theme are lifted by a soft shadow
    '--card-lift': 'var(--shadow-2)',
  }
}

/** Of a gradient's two stops, the one a coloured name reads worst on: the lighter on a dark card, the darker on a
 *  light one (names are moved toward white on dark surfaces and toward black on light ones). */
export function nameStop(top: string, bottom: string, light: boolean): string {
  const l = (c: string) => luminance(parseHex(c) ?? [0, 0, 0])
  return (l(top) > l(bottom)) !== light ? top : bottom
}
