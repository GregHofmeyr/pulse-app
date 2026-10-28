// Card themes (plan reconciliation 8): two raw colours, mixed into near-black so card text always reads.
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

/** The CSS custom properties a card (or its preview) sets on its root. */
export function cardVars(top: string, bottom: string): Record<string, string> {
  return {
    '--card-top': mixHex(top, NEAR_BLACK, CARD_MIX),
    '--card-bottom': mixHex(bottom, NEAR_BLACK, CARD_MIX),
    '--panel-top': mixHex(top, NEAR_BLACK, PANEL_MIX),
    '--panel-bottom': mixHex(bottom, NEAR_BLACK, PANEL_MIX),
    '--glow-top': top,
    '--glow-bottom': bottom,
    '--card-text': CARD_TEXT,
    '--card-text-2': CARD_TEXT_2,
  }
}
