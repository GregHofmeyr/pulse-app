import { describe, expect, it } from 'vitest'
import { BODY_RATIO, cardInk, cardVars, composite, DARK_CARD, DEFAULT_THEME, EDGE_SHADE, LIGHT_CARD, mixHex, nameStop, presetFor, THEME_PRESETS } from './cardTheme'
import { contrast, luminance, parseHex } from './color'

const lum = (c: string) => luminance(parseHex(c)!)
/** Every surface the card's text sits on: its two ends and the panels and the message box over them. */
const surfaces = (v: Record<string, string>, ink: typeof LIGHT_CARD) =>
  [v['--card-top'], v['--card-bottom']].flatMap((s) => [s, composite(s, ink.panel), composite(s, ink.field)])

// The owner's references (#156): pastel lilac, mint to teal, white to periwinkle, sky blue, pink, cyan to lilac,
// lavender-white, dusty rose. Made-up hexes of those kinds.
const PASTELS: [string, string][] = [
  ['#e9d5f7', '#cdb4ee'], ['#cdeedd', '#8fd3c7'], ['#ffffff', '#b4c2f7'], ['#bfe3ff', '#9ccdf5'],
  ['#ffc8e0', '#f5a9cb'], ['#b8f1f5', '#d6c4f5'], ['#f4f0ff', '#e6defa'], ['#efd0d3', '#e0b4ba'],
]
const SWEEP = ['#ffffff', '#ffb6d9', '#ffff00', '#00ffff', '#00ff00', '#8a3cff', '#ff8a2a', '#808080', '#3a3d46', '#0d1430', '#000000', '#c2457a']

describe('cardTheme', () => {
  it('presets are exact', () => {
    expect(THEME_PRESETS).toEqual([
      { key: 'sunset', label: 'Sunset', top: '#8a3cff', bottom: '#ff8a2a' },
      { key: 'night', label: 'Night sky', top: '#22325e', bottom: '#0d1430' },
      { key: 'forest', label: 'Forest', top: '#1f7a5a', bottom: '#0f2b3a' },
      { key: 'berry', label: 'Berry', top: '#c2457a', bottom: '#3a1f5e' },
      { key: 'plain', label: 'Plain', top: '#3a3d46', bottom: '#16171b' },
    ])
    expect(DEFAULT_THEME.key).toBe('plain')
  })

  it('presetFor finds presets and returns null for custom', () => {
    expect(presetFor('#8a3cff', '#ff8a2a')).toBe('sunset')
    expect(presetFor('#8A3CFF', '#FF8A2A')).toBe('sunset')
    expect(presetFor('#3a3d46', '#16171b')).toBe('plain')
    expect(presetFor('#8a3cff', '#000000')).toBeNull()
  })

  it('cardVars exposes the properties', () => {
    expect(Object.keys(cardVars('#ffffff', '#000000')).sort()).toEqual([
      '--card-border-bottom', '--card-border-top', '--card-bottom', '--card-danger', '--card-edge', '--card-field', '--card-hover', '--card-lift', '--card-line',
      '--card-ok', '--card-panel', '--card-skeleton', '--card-text', '--card-text-2', '--card-top',
      '--glow-bottom', '--glow-bottom-a', '--glow-top', '--glow-top-a', '--panel-bottom', '--panel-top',
    ])
  })
})

describe('the card is drawn in its own two colours (#156)', () => {
  it('pastel colours are used as picked, with near-black text', () => {
    for (const [top, bottom] of PASTELS) {
      const v = cardVars(top, bottom)
      expect([v['--card-top'], v['--card-bottom']], `${top} ${bottom}`).toEqual([top, bottom])
      expect(cardInk(top, bottom).light).toBe(true)
      expect(lum(v['--card-text'])).toBeLessThan(0.02)
      // secondary text is a darker grey, not the app's light-on-dark tokens
      expect(lum(v['--card-text-2'])).toBeLessThan(0.1)
      expect(lum(v['--card-text-2'])).toBeGreaterThan(lum(v['--card-text']))
    }
  })

  it('dark colours are used as picked, with light text as before', () => {
    for (const p of THEME_PRESETS.slice(1)) {
      const v = cardVars(p.top, p.bottom)
      expect(cardInk(p.top, p.bottom).light).toBe(false)
      expect([v['--card-text'], v['--card-text-2']]).toEqual([DARK_CARD.text, DARK_CARD.text2])
      // a mid-dark end (Forest's green, Berry's pink) darkens a little so the secondary grey reads; the rest as picked
      for (const [drawn, picked] of [[v['--card-top'], p.top], [v['--card-bottom'], p.bottom]]) {
        expect(lum(drawn)).toBeLessThanOrEqual(lum(picked))
        expect(lum(drawn)).toBeGreaterThan(lum(picked) * 0.6)
      }
    }
    for (const p of [THEME_PRESETS[1], THEME_PRESETS[4]]) expect(cardInk(p.top, p.bottom)).toEqual({ light: false, top: p.top, bottom: p.bottom })
  })

  it('the app theme changes nothing but the lift', () => {
    for (const [top, bottom] of [...PASTELS, ['#22325e', '#0d1430'] as [string, string]]) {
      const dark = cardVars(top, bottom, { light: false })
      const light = cardVars(top, bottom, { light: true })
      expect({ ...light, '--card-lift': '' }).toEqual({ ...dark, '--card-lift': '' })
      expect(light['--card-lift']).toBe('var(--shadow-2)')
      expect(dark['--card-lift']).toBe('0 0 #0000')
    }
  })

  it('whatever the colours, body text reads (WCAG AA) on every surface of the card', () => {
    for (const top of SWEEP) for (const bottom of SWEEP) {
      const v = cardVars(top, bottom)
      const ink = cardInk(top, bottom).light ? LIGHT_CARD : DARK_CARD
      for (const s of surfaces(v, ink)) {
        expect(contrast(v['--card-text'], s), `${top} ${bottom} ${s}`).toBeGreaterThanOrEqual(BODY_RATIO)
        expect(contrast(v['--card-text-2'], s), `2 ${top} ${bottom} ${s}`).toBeGreaterThanOrEqual(BODY_RATIO)
        expect(contrast(v['--card-ok'], s), `ok ${top} ${bottom} ${s}`).toBeGreaterThanOrEqual(BODY_RATIO)
        expect(contrast(v['--card-danger'], s), `danger ${top} ${bottom} ${s}`).toBeGreaterThanOrEqual(BODY_RATIO)
      }
    }
  })

  it('a pair neither text reads on as is (purple to orange) moves the least it must, toward one side', () => {
    const { light, top, bottom } = cardInk('#8a3cff', '#ff8a2a')
    const moved = [top, bottom]
    // both ends move the same way: lighter for dark text, darker for light text
    for (const [c, was] of [[moved[0], '#8a3cff'], [moved[1], '#ff8a2a']]) {
      if (light) expect(lum(c)).toBeGreaterThanOrEqual(lum(was))
      else expect(lum(c)).toBeLessThanOrEqual(lum(was))
    }
    // one step less and the text would not read: compare with the other look's own minimum
    const v = cardVars('#8a3cff', '#ff8a2a')
    expect(contrast(v['--card-text'], v['--card-top'])).toBeGreaterThanOrEqual(BODY_RATIO)
    expect(contrast(v['--card-text'], v['--card-top'])).toBeLessThan(BODY_RATIO + 2.5)
  })

  it('the panels and the message box are translucent overlays; the solid panel colours are what they make', () => {
    const v = cardVars('#e9d5f7', '#cdb4ee')
    expect(v['--card-panel']).toBe('rgb(255 255 255 / .4)')
    expect(v['--card-field']).toBe('rgb(0 0 0 / .07)')
    expect(v['--panel-top']).toBe(composite('#e9d5f7', LIGHT_CARD.panel))
    const d = cardVars('#22325e', '#0d1430')
    expect(d['--card-panel']).toMatch(/^rgb\(0 0 0 \/ \.\d+\)$/)
    expect(d['--card-line']).toBe('rgb(242 243 246 / .1)')
    expect(v['--card-line']).toBe('rgb(23 24 28 / .1)')
  })

  it('the border is a stronger shade of each colour: darker on a light card, lighter on a dark one', () => {
    const v = cardVars('#ffffff', '#b4c2f7')
    expect(v['--card-border-top']).toBe(mixHex('#000000', '#ffffff', EDGE_SHADE))
    expect(lum(v['--card-border-bottom'])).toBeLessThan(lum('#b4c2f7'))
    const d = cardVars('#22325e', '#0d1430')
    expect(lum(d['--card-border-top'])).toBeGreaterThan(lum('#22325e'))
  })

  it('the glow is each raw colour at 45%', () => {
    const v = cardVars('#FFC8E0', '#f5a9cb')
    expect([v['--glow-top'], v['--glow-bottom']]).toEqual(['#ffc8e0', '#f5a9cb'])
    expect([v['--glow-top-a'], v['--glow-bottom-a']]).toEqual(['#ffc8e073', '#f5a9cb73'])
  })
})

describe('nameStop (#139 review: a name reads on both ends of the card gradient)', () => {
  it('the lighter stop on a dark card, the darker on a light one', () => {
    expect(nameStop('#202020', '#505050', false)).toBe('#505050')
    expect(nameStop('#505050', '#202020', false)).toBe('#505050')
    expect(nameStop('#f0f0f0', '#c0c0c0', true)).toBe('#c0c0c0')
    expect(nameStop('#c0c0c0', '#f0f0f0', true)).toBe('#c0c0c0')
  })
})
