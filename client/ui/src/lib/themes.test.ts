import { describe, expect, it } from 'vitest'
import { contrast, parseHex, readable } from './color'
import { ACCENT_PRESETS, DARK_INK, deriveAccent, NAME_LEVELS, surfacesFor, textSurfaces, THEMES, TINTS, type ThemeKey, type TintKey } from './themes'

/** HSL hue in degrees (null for a grey). */
function hue(hex: string): number | null {
  const [r, g, b] = parseHex(hex)!.map((v) => v / 255)
  const max = Math.max(r, g, b)
  const min = Math.min(r, g, b)
  if (max - min < 1e-6) return null
  const d = max - min
  const h = max === r ? ((g - b) / d) % 6 : max === g ? (b - r) / d + 2 : (r - g) / d + 4
  return (h * 60 + 360) % 360
}
const hueGap = (a: number, b: number) => Math.min(Math.abs(a - b), 360 - Math.abs(a - b))

/** Every theme, and Light with every tint. */
const VARIANTS: { theme: ThemeKey; tint: TintKey }[] = THEMES.flatMap((t) => (t.key === 'light' ? TINTS.map((n) => ({ theme: t.key, tint: n.key })) : [{ theme: t.key, tint: 'plain' as TintKey }]))

describe('readable', () => {
  it('leaves a colour that already reaches the ratio exactly as it is', () => {
    expect(readable('#8b9cff', ['#212228'], 4.5)).toBe('#8b9cff')
    expect(readable('#1a1a1a', ['#ffffff'], 7)).toBe('#1a1a1a')
  })

  it('darkens on a light surface and lightens on a dark one, just enough', () => {
    const onWhite = readable('#ffc4a3', ['#ffffff'], 4.5)
    expect(contrast(onWhite, '#ffffff')).toBeGreaterThanOrEqual(4.5)
    expect(contrast(onWhite, '#ffffff')).toBeLessThan(4.9) // 1% steps: no overshoot to black
    const onBlack = readable('#3a0a6a', ['#000000'], 4.5)
    expect(contrast(onBlack, '#000000')).toBeGreaterThanOrEqual(4.5)
    expect(contrast(onBlack, '#000000')).toBeLessThan(4.9)
  })

  it('keeps the hue of a saturated colour', () => {
    for (const c of ['#ff7fae', '#8fd0ff', '#a2f7c2', '#ffe58f', '#ff0000', '#3d5afe']) {
      for (const bg of ['#ffffff', '#000000', '#212228']) {
        const out = readable(c, [bg], 7)
        const h = hue(out)
        if (h !== null) expect(hueGap(h, hue(c)!), `${c} on ${bg} → ${out}`).toBeLessThanOrEqual(2)
      }
    }
  })

  it('meets the ratio on every surface it is given', () => {
    const out = readable('#8b9cff', ['#ffffff', '#e9eaed', '#ececef'], 4.5)
    for (const bg of ['#ffffff', '#e9eaed', '#ececef']) expect(contrast(out, bg)).toBeGreaterThanOrEqual(4.5)
  })

  it('gives back the input untouched when it is not a colour', () => {
    expect(readable('red', ['#ffffff'], 4.5)).toBe('red')
  })
})

describe('the theme table', () => {
  it('has the eight themes, Pulse Dark first (the default); the owner kept all three warm ones', () => {
    expect(THEMES.map((t) => t.key)).toEqual(['dark', 'midnight', 'oled', 'sunset', 'ember', 'plum', 'pastel', 'light'])
    expect(THEMES[0].label).toBe('Pulse Dark')
    expect(TINTS.map((t) => t.key)).toEqual(['plain', 'pink', 'orange', 'blue', 'green'])
  })

  it('a tint only changes Light', () => {
    expect(surfacesFor('dark', 'pink')).toEqual(surfacesFor('dark', 'plain'))
    expect(surfacesFor('light', 'pink').bg0).not.toBe(surfacesFor('light', 'plain').bg0)
    // raised surfaces stay white whatever the tint
    for (const t of TINTS) expect([surfacesFor('light', t.key).bg2, surfacesFor('light', t.key).menu]).toEqual(['#ffffff', '#ffffff'])
  })

  it('an unknown theme or tint falls back to Pulse Dark / plain', () => {
    expect(surfacesFor('nebula' as ThemeKey, 'plain')).toEqual(surfacesFor('dark', 'plain'))
    expect(surfacesFor('light', 'teal' as TintKey)).toEqual(surfacesFor('light', 'plain'))
  })

  it('the readability levels are Low 3:1, Normal 4.5:1, High 7:1', () => {
    expect(NAME_LEVELS.map((l) => [l.key, l.label, l.ratio])).toEqual([['low', 'Low', 3], ['normal', 'Normal', 4.5], ['high', 'High', 7]])
  })
})

describe('deriveAccent', () => {
  it("Pulse Dark's own accent derives today's tokens exactly", () => {
    expect(deriveAccent('#8b9cff', surfacesFor('dark', 'plain'), false)).toEqual({
      '--accent': '#8b9cff',
      '--accent-soft': 'rgba(139, 156, 255, .15)',
      '--accent-faint': 'rgba(139, 156, 255, .08)',
      '--accent-mid': 'rgba(139, 156, 255, .22)',
      '--accent-strong': 'rgba(139, 156, 255, .3)',
      '--focus-soft': 'rgba(139, 156, 255, .45)',
      '--focus-ring': '#8b9cff',
      '--on-accent': DARK_INK,
      '--accent-text': '#8b9cff',
    })
  })

  const SWEEP = ['#ffffff', '#fffde0', '#f0f0f0', '#808080', '#050505', '#000000', '#ff0000', '#00ff00', '#0000ff', '#ffff00', '#7f7f00']
  const ALL = [...ACCENT_PRESETS.map((p) => p.color), ...THEMES.map((t) => t.accent), ...SWEEP]
  it.each(VARIANTS)('$theme ($tint): text on accent, accent text and the focus ring stay readable for every preset and odd colours', ({ theme, tint }) => {
    const s = surfacesFor(theme, tint)
    const light = THEMES.find((t) => t.key === theme)!.light
    for (const c of ALL) {
      const a = deriveAccent(c, s, light)
      expect(a['--accent']).toBe(c)
      // dark ink or white, whichever reads better, never under 4.5:1
      expect(contrast(a['--on-accent'], c), `${theme} ${c}`).toBeGreaterThanOrEqual(Math.max(4.5, Math.min(contrast(DARK_INK, c), contrast('#ffffff', c))))
      for (const bg of textSurfaces(s)) {
        expect(contrast(a['--accent-text'], bg), `${theme}/${tint} accent text ${c} on ${bg}`).toBeGreaterThanOrEqual(4.5)
        expect(contrast(a['--focus-ring'], bg), `${theme}/${tint} focus ring ${c} on ${bg}`).toBeGreaterThanOrEqual(3)
      }
      expect(a['--accent-soft']).toMatch(/^rgba\(\d+, \d+, \d+, \.1[45]\)$/)
    }
  })

  it('text on the accent reads at 4.5:1 for every preset and any colour (pure black when neither ink nor white does)', () => {
    const greys = Array.from({ length: 52 }, (_, i) => '#' + (i * 5).toString(16).padStart(2, '0').repeat(3))
    for (const c of [...ACCENT_PRESETS.map((p) => p.color), ...SWEEP, ...greys, '#777777', '#3d7a8a', '#b04080']) {
      const on = deriveAccent(c, surfacesFor('dark', 'plain'), false)['--on-accent']
      expect(['#141519', '#ffffff', '#000000']).toContain(on)
      expect(contrast(on, c), c).toBeGreaterThanOrEqual(4.5)
    }
  })

  it('light themes use slightly different translucencies (a stronger focus edge, a fainter tint)', () => {
    const a = deriveAccent('#8b9cff', surfacesFor('light', 'plain'), true)
    expect([a['--accent-soft'], a['--accent-faint'], a['--focus-soft']]).toEqual(['rgba(139, 156, 255, .14)', 'rgba(139, 156, 255, .07)', 'rgba(139, 156, 255, .55)'])
  })
})
