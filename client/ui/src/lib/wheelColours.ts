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
