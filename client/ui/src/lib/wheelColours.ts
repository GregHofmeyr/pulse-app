// The picker wheel's slice colours (#158): the one place they are made. The owner chose the playground's "Neon
// spectrum": fully saturated hues evenly spaced round the wheel, hsl(360·i/n 100% 58%) for slice i of n. They are
// content colours generated per wheel, not UI: a theme can't recolour them, which is why they aren't app.css tokens
// (colorTokens.test.ts names this file as the exception). Each label is white where white reads on its slice (3:1,
// as bold labels need) and near-black elsewhere, so every label reads on every hue.

/** Near-black label text for the light hues (yellows, greens, cyans). */
export const LABEL_DARK = '#15161a'
export const LABEL_LIGHT = '#ffffff'

const LIGHTNESS = 0.58

/** Slice i of n's hue in degrees. */
export const sliceHue = (i: number, n: number): number => (n > 0 ? (360 * i) / n : 0)

/** Slice i of n's colour, as CSS. */
export function sliceColour(i: number, n: number): string {
  return `hsl(${round(sliceHue(i, n))}, 100%, 58%)`
}

/** The label colour for slice i of n. */
export function labelColour(i: number, n: number): string {
  return contrast(LABEL_LIGHT, sliceRgb(i, n)) >= 3 ? LABEL_LIGHT : LABEL_DARK
}

const round = (v: number) => Math.round(v * 100) / 100

/** Slice i of n as sRGB 0..255 (HSL with full saturation). */
export function sliceRgb(i: number, n: number): [number, number, number] {
  const h = sliceHue(i, n) / 360
  const l = LIGHTNESS
  const q = l < 0.5 ? l * 2 : l + 1 - l
  const p = 2 * l - q
  const ch = (t: number) => {
    const u = t < 0 ? t + 1 : t > 1 ? t - 1 : t
    if (u < 1 / 6) return p + (q - p) * 6 * u
    if (u < 1 / 2) return q
    if (u < 2 / 3) return p + (q - p) * (2 / 3 - u) * 6
    return p
  }
  return [ch(h + 1 / 3), ch(h), ch(h - 1 / 3)].map((v) => Math.round(v * 255)) as [number, number, number]
}

function luminance([r, g, b]: [number, number, number]): number {
  const lin = (c: number) => {
    const s = c / 255
    return s <= 0.03928 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4
  }
  return 0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b)
}

function hexRgb(hex: string): [number, number, number] {
  const v = parseInt(hex.slice(1), 16)
  return [(v >> 16) & 255, (v >> 8) & 255, v & 255]
}

/** WCAG contrast ratio between a hex colour and an sRGB one. */
export function contrast(hex: string, rgb: [number, number, number]): number {
  const a = luminance(hexRgb(hex))
  const b = luminance(rgb)
  return (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05)
}

// --- Palettes (#259): a saved wheel may swap the neon spectrum for a named set of theme colours. These are app.css
// tokens (CSS `var()`s), so a theme recolours them like the rest of the app and they need no literal here. Their
// labels are the video chrome's white with a dark edge (`--on-video` over `--video-chrome-strong`), which reads on
// any fill in every theme, since a token's value isn't known until the theme is.

export type PaletteId = 'neon' | 'gems' | 'signal' | 'meadow' | 'accent'

/** The palettes a wheel can use, in the order the picker lists them (`neon` first: the default). Names match the
 *  server's `wheel_palettes`. */
export const PALETTES: readonly { id: PaletteId; label: string; tokens: readonly string[] }[] = [
  { id: 'neon', label: 'Neon', tokens: [] },
  // the badge rarities
  { id: 'gems', label: 'Gems', tokens: ['--rarity-common', '--rarity-uncommon', '--rarity-rare', '--rarity-epic', '--rarity-legendary', '--rarity-mythical'] },
  { id: 'signal', label: 'Signal', tokens: ['--ok', '--warn', '--danger', '--accent'] },
  { id: 'meadow', label: 'Meadow', tokens: ['--quote-green', '--quote-green-mid', '--quote-green-deep', '--ok-deep'] },
  { id: 'accent', label: 'Accent', tokens: ['--accent', '--bg-4'] },
]

/** A stored palette name as one of ours; anything else (none, or a newer one) is the neon spectrum. */
export const paletteOf = (name: string | null | undefined): PaletteId => PALETTES.find((p) => p.id === name)?.id ?? 'neon'

/** Which of a palette's tokens slice i of n takes: round the palette, except that the last slice never matches the
 *  first (they touch). */
function tokenIndex(i: number, n: number, len: number): number {
  const k = i % len
  return n > 1 && i === n - 1 && k === 0 && len > 1 ? 1 % len : k
}

/** Slice i of n's fill and label in a palette: the neon spectrum's colours, or a palette's tokens with an edged label
 *  (`edged`: draw the label with a `--video-chrome-strong` outline). */
export function sliceStyle(i: number, n: number, palette: string | null | undefined): { fill: string; ink: string; edged: boolean } {
  const p = PALETTES.find((x) => x.id === paletteOf(palette))!
  if (!p.tokens.length) return { fill: sliceColour(i, n), ink: labelColour(i, n), edged: false }
  return { fill: `var(${p.tokens[tokenIndex(i, n, p.tokens.length)]})`, ink: 'var(--on-video)', edged: true }
}
