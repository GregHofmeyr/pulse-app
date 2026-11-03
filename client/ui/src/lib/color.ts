// Colour maths for themes, the accent and names (#42): parse/format `#rrggbb`, WCAG luminance and contrast, and
// `readable`, the one function that moves a colour just far enough to read on a surface.

export type Rgb = [number, number, number]

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

/** WCAG contrast ratio of two `#rrggbb` colours. */
export function contrast(a: string, b: string): number {
  const la = luminance(parseHex(a)!)
  const lb = luminance(parseHex(b)!)
  return (Math.max(la, lb) + 0.05) / (Math.min(la, lb) + 0.05)
}

/** `c` mixed `k`% (0..100) toward `to`, each channel rounded. */
export function mixRgb(c: Rgb, to: Rgb, k: number): Rgb {
  return c.map((v, i) => Math.floor(v + ((to[i] - v) * k) / 100 + 0.5)) as Rgb
}

/** `rgba(r, g, b, a)` of a `#rrggbb` colour, with the alpha written like app.css writes it (`.15`). */
export function rgba(hex: string, alpha: number): string {
  const [r, g, b] = parseHex(hex)!
  return `rgba(${r}, ${g}, ${b}, ${String(alpha).replace(/^0\./, '.')})`
}

// Below this luminance white gives more contrast than black (the two meet at about 0.179).
const DARK_BELOW = 0.179

/** True when white reads better on `hex` than black does. */
export function isDark(hex: string): boolean {
  const c = parseHex(hex)
  return !!c && luminance(c) < DARK_BELOW
}

/**
 * `fg` unchanged if it already reaches `ratio` on every surface in `bgs`, else mixed toward white (dark surfaces) or
 * black (light ones) in 1% steps to the smallest mix that does. Mixing toward white or black keeps the hue. All of
 * `bgs` are expected on the same side (a theme's surfaces); the first decides the direction. Not a colour: unchanged.
 */
export function readable(fg: string, bgs: string[], ratio: number): string {
  const c = parseHex(fg)
  if (!c || !bgs.length) return fg
  const target: Rgb = isDark(bgs[0]) ? [255, 255, 255] : [0, 0, 0]
  const ok = (hex: string) => bgs.every((bg) => contrast(hex, bg) >= ratio)
  for (let k = 0; k <= 100; k++) {
    const m = toHex(mixRgb(c, target, k))
    if (ok(m)) return k === 0 ? fg : m
  }
  return toHex(target)
}
