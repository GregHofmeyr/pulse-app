// Speaking rings (#651): the styles and colours someone can pick for the ring others see while they talk. The keys
// are the server's (`RING_STYLES` / `RING_COLORS` in server/src/profiles/mod.rs); a literal copy is in
// speakingRing.test.ts.
import { parseHex, readable, toHex } from './color'
import type { User } from './protocol/User'

export type RingStyle =
  | 'classic'
  | 'glow'
  | 'pulse'
  | 'ripple'
  | 'sweep'
  | 'orbit'
  | 'halo'
  | 'viewfinder'
  | 'liquid'
  | 'dots'
  | 'bars'

export type RingColor = 'green' | 'blue' | 'gold' | 'pink' | 'cyan' | 'orange' | 'white' | 'purple' | 'profile'

/** Every style in picker order. `reactive`: it follows the speaker's loudness. */
export const RING_STYLES: { key: RingStyle; label: string; reactive: boolean }[] = [
  { key: 'classic', label: 'Classic', reactive: false },
  { key: 'glow', label: 'Soft glow', reactive: false },
  { key: 'pulse', label: 'Pulse', reactive: false },
  { key: 'ripple', label: 'Ripple', reactive: false },
  { key: 'sweep', label: 'Sweep', reactive: false },
  { key: 'orbit', label: 'Orbit', reactive: false },
  { key: 'halo', label: 'Halo', reactive: false },
  { key: 'viewfinder', label: 'Viewfinder', reactive: false },
  { key: 'liquid', label: 'Liquid', reactive: true },
  { key: 'dots', label: 'Wave of dots', reactive: true },
  { key: 'bars', label: 'Bars round the ring', reactive: true },
]

/** Every colour in picker order. `profile`: the first colour of the person's name style. */
export const RING_COLORS: { key: RingColor; label: string }[] = [
  { key: 'green', label: 'Green' },
  { key: 'blue', label: 'Pulse blue' },
  { key: 'gold', label: 'Gold' },
  { key: 'pink', label: 'Pink' },
  { key: 'cyan', label: 'Cyan' },
  { key: 'orange', label: 'Orange' },
  { key: 'white', label: 'White' },
  { key: 'purple', label: 'Purple' },
  { key: 'profile', label: 'Profile colour' },
]

/** Where a ring shows: the voice stage, the full view's strip, the call band, the channel list (22px, always still)
 * and the Profile settings preview. */
export type RingPlace = 'stage' | 'strip' | 'band' | 'list' | 'preview'

const STYLE_KEYS = new Set<string>(RING_STYLES.map((s) => s.key))
const COLOR_KEYS = new Set<string>(RING_COLORS.map((c) => c.key))

/** Someone's ring style: Classic when absent, unknown (a newer server's key) or emptied for a blocked viewer. */
export function ringStyleOf(u: User | null): RingStyle {
  const k = u?.ring_style
  return k && STYLE_KEYS.has(k) ? (k as RingStyle) : 'classic'
}

/** Someone's ring colour: green when absent, unknown or emptied. */
export function ringColorOf(u: User | null): RingColor {
  const k = u?.ring_color
  return k && COLOR_KEYS.has(k) ? (k as RingColor) : 'green'
}

/** True for Liquid, Wave of dots and Bars: they follow the speaker's loudness. */
export function isReactive(style: RingStyle): boolean {
  return RING_STYLES.some((s) => s.key === style && s.reactive)
}

/** The CSS colour for `--speak`: the palette key's theme token, or for `profile` the first name colour made readable
 * (3:1) on `bg`, the surface the ring sits on; green without one. */
export function speakCss(u: User | null, color: RingColor, bg: string): string {
  if (color !== 'profile') return `var(--speak-${color})`
  const first = u?.style.colors[0]
  const rgb = first ? parseHex(first) : null
  if (!rgb) return 'var(--speak-green)'
  return readable(toHex(rgb), parseHex(bg) ? [bg] : [], 3)
}

/** `off`: not speaking, every layer hidden and still. `on`: animated. `still`: lit, shown as the style's still version. */
export type RingState = 'off' | 'on' | 'still'

/** The style actually drawn: "Show everyone's rings as Classic" forces Classic (the colour stays). */
export function effectiveStyle(style: RingStyle, o: { classicForAll: boolean }): RingStyle {
  return o.classicForAll ? 'classic' : style
}

/** `animating`: within the 8-ring cap. The channel list is always still. */
export function ringState(o: { lit: boolean; place: RingPlace; style: RingStyle; reducedMotion: boolean; streaming: boolean; animating: boolean }): RingState {
  if (!o.lit) return 'off'
  if (o.place === 'list' || o.reducedMotion || o.streaming || !o.animating) return 'still'
  return 'on'
}

/** Whether a visible ring of this (effective) style asks the core for levels: reactive, moving, and a call tile (the
 * preview fakes its own; the list is still). */
export function watchesLevels(style: RingStyle, o: { reducedMotion: boolean; streaming: boolean; place: RingPlace }): boolean {
  return isReactive(style) && !o.reducedMotion && !o.streaming && o.place !== 'list' && o.place !== 'preview'
}
