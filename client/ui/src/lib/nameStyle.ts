// The one name-style resolver (plan reconciliations 7, 11, 12). The lightening rule has a twin in
// server/src/profiles/colors.rs; both must give identical results.
import { fontByKey, type NameFont } from './nameFonts'
import type { User } from './protocol/User'

export const MIN_NAME_LUMINANCE = 0.39

type Rgb = [number, number, number]

/** `#rrggbb`, case-insensitive, and nothing else. */
export function parseHex(s: string): Rgb | null {
  if (!/^#[0-9a-fA-F]{6}$/.test(s)) return null
  return [1, 3, 5].map((i) => parseInt(s.slice(i, i + 2), 16)) as Rgb
}

/** `#rrggbb`, lowercase. */
export function toHex(rgb: Rgb): string {
  return '#' + rgb.map((v) => v.toString(16).padStart(2, '0')).join('')
}

/** WCAG relative luminance. */
export function luminance(rgb: Rgb): number {
  const lin = (c: number) => {
    const v = c / 255
    return v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4
  }
  return 0.2126 * lin(rgb[0]) + 0.7152 * lin(rgb[1]) + 0.0722 * lin(rgb[2])
}

/** `hex` unchanged if bright enough for a name, else mixed towards white in 1% steps to the smallest mix that is. */
export function lightenForName(hex: string): string {
  const c = parseHex(hex)
  if (!c) return hex
  if (luminance(c) >= MIN_NAME_LUMINANCE) return toHex(c)
  for (let k = 1; k <= 100; k++) {
    const m = c.map((v) => Math.floor(v + ((255 - v) * k) / 100 + 0.5)) as Rgb
    if (luminance(m) >= MIN_NAME_LUMINANCE) return toHex(m)
  }
  return '#ffffff'
}

/** WCAG contrast ratio of two `#rrggbb` colours. */
export function contrast(a: string, b: string): number {
  const la = luminance(parseHex(a)!)
  const lb = luminance(parseHex(b)!)
  return (Math.max(la, lb) + 0.05) / (Math.min(la, lb) + 0.05)
}

export type NamePreset = { key: string; label: string; colors: string[] }

export const NAME_SOLIDS: NamePreset[] = [
  { key: 'peach', label: 'Peach', colors: ['#ffc4a3'] },
  { key: 'lilac', label: 'Lilac', colors: ['#d4c2ff'] },
  { key: 'mint', label: 'Mint', colors: ['#a8e6cf'] },
  { key: 'sky', label: 'Sky', colors: ['#a3d8ff'] },
]

export const NAME_GRADIENTS: NamePreset[] = [
  { key: 'sunset', label: 'Sunset', colors: ['#ffbe7a', '#ff7fae'] },
  { key: 'ice', label: 'Ice', colors: ['#8fd0ff', '#c9a8ff'] },
  { key: 'mint', label: 'Mint', colors: ['#a2f7c2', '#8fd2ff'] },
  { key: 'peach', label: 'Peach', colors: ['#ffe58f', '#ff9d96'] },
  { key: 'aurora', label: 'Aurora', colors: ['#f6b6ff', '#9fb4ff'] },
]

export type Paint = { kind: 'plain' } | { kind: 'solid'; color: string } | { kind: 'gradient'; from: string; to: string }

/** A role colour replaces the colour and keeps the font. Styles off means the default font and colour, but a role colour still shows. */
export function resolveNameStyle(user: User | null, opts: { roleColor: string | null; showStyles: boolean }): { font: NameFont | null; paint: Paint } {
  const font = opts.showStyles && user ? fontByKey(user.style.font) : null
  if (opts.roleColor && parseHex(opts.roleColor)) return { font, paint: { kind: 'solid', color: lightenForName(opts.roleColor) } }
  const colors = opts.showStyles && user ? user.style.colors.filter((c) => parseHex(c)).map(lightenForName) : []
  if (colors.length >= 2) return { font, paint: { kind: 'gradient', from: colors[0], to: colors[1] } }
  if (colors.length === 1) return { font, paint: { kind: 'solid', color: colors[0] } }
  return { font, paint: { kind: 'plain' } }
}

/** The inline style that draws a resolved name. */
export function nameCss(r: { font: NameFont | null; paint: Paint }): string {
  const parts: string[] = []
  if (r.font) parts.push(`font-family: ${r.font.family}`, `font-weight: ${r.font.weight}`, `--name-scale: ${r.font.scale}`)
  else parts.push('--name-scale: 1')
  if (r.paint.kind === 'solid') parts.push(`color: ${r.paint.color}`)
  else if (r.paint.kind === 'gradient') {
    parts.push(`background-image: linear-gradient(90deg, ${r.paint.from}, ${r.paint.to})`, '-webkit-background-clip: text', 'background-clip: text', 'color: transparent')
  }
  return parts.join('; ')
}
