// The one name-style resolver (plan reconciliations 7, 11, 12). The lightening rule has a twin in
// server/src/profiles/colors.rs; both must give identical results.
import { contrast, isDark, luminance, parseHex, readable, toHex, type Rgb } from './color'
import { fontByKey, type NameFont } from './nameFonts'
import type { User } from './protocol/User'

export const MIN_NAME_LUMINANCE = 0.39

export { contrast, luminance, parseHex, toHex }

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

/** A name colour that reaches `ratio` on `bg`. On a dark surface it's also never darker than today's rule
 *  ({@link lightenForName}; owner, 2026-11-03: nobody asked for darker names): the brighter of the two. */
function readableName(c: string, bg: string, ratio: number): string {
  const r = readable(c, [bg], ratio)
  if (!isDark(bg)) return r
  const today = lightenForName(c)
  return luminance(parseHex(today)!) >= luminance(parseHex(r)!) ? today : r
}

export type Paint = { kind: 'plain' } | { kind: 'solid'; color: string } | { kind: 'gradient'; from: string; to: string }

/**
 * A role colour replaces the colour and keeps the font. Styles off means the default font and colour, but a role colour still shows.
 * Given the surface the name sits on (`bg`) and the Name readability contrast (`ratio`), every colour (a role colour, a
 * solid, each gradient stop) keeps its hue and moves just far enough to reach that contrast there (#42), and on a dark
 * surface is never darker than the old rule (luminance 0.39); without them, that old rule alone.
 */
export function resolveNameStyle(user: User | null, opts: { roleColor: string | null; showStyles: boolean; bg?: string; ratio?: number }): { font: NameFont | null; paint: Paint } {
  const font = opts.showStyles && user ? fontByKey(user.style.font) : null
  const fix = opts.bg && parseHex(opts.bg) ? (c: string) => readableName(c, opts.bg!, opts.ratio ?? 4.5) : lightenForName
  if (opts.roleColor && parseHex(opts.roleColor)) return { font, paint: { kind: 'solid', color: fix(opts.roleColor) } }
  const colors = opts.showStyles && user ? user.style.colors.filter((c) => parseHex(c)).map(fix) : []
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
