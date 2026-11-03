import { describe, expect, it } from 'vitest'
import { CARD_MIX, CARD_MIX_LIGHT, CARD_TEXT, CARD_TEXT_2, cardVars, DEFAULT_THEME, mixHex, NEAR_BLACK, NEAR_WHITE, nameStop, PANEL_MIX, PANEL_MIX_LIGHT, presetFor, THEME_PRESETS } from './cardTheme'
import { contrast, lightenForName } from './nameStyle'

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
    expect([CARD_MIX, PANEL_MIX, NEAR_BLACK, CARD_TEXT, CARD_TEXT_2]).toEqual([0.3, 0.18, '#0d0e12', '#f2f3f6', '#d6d8df'])
  })

  it('worst themes keep text readable', () => {
    for (const top of ['#ffffff', '#ffff00', '#00ffff', '#00ff00']) for (const bottom of ['#ffffff', '#ffff00', '#00ffff', '#00ff00']) {
      const v = cardVars(top, bottom)
      for (const surface of [v['--card-top'], v['--card-bottom']]) {
        expect(contrast(CARD_TEXT, surface), surface).toBeGreaterThanOrEqual(4.5)
        expect(contrast(CARD_TEXT_2, surface), surface).toBeGreaterThanOrEqual(4.5)
        expect(contrast(lightenForName('#000000'), surface), surface).toBeGreaterThanOrEqual(3.0)
      }
      for (const panel of [v['--panel-top'], v['--panel-bottom']]) expect(contrast(CARD_TEXT_2, panel), panel).toBeGreaterThanOrEqual(4.5)
    }
  })

  it('cardVars exposes the properties', () => {
    const v = cardVars('#ffffff', '#000000')
    expect(Object.keys(v).sort()).toEqual([
      '--card-bottom', '--card-danger', '--card-edge', '--card-hover', '--card-lift', '--card-line', '--card-ok', '--card-skeleton', '--card-text', '--card-text-2', '--card-top',
      '--glow-bottom', '--glow-bottom-a', '--glow-top', '--glow-top-a', '--panel-bottom', '--panel-top',
    ])
    expect(v['--glow-top']).toBe('#ffffff')
    expect(v['--card-top']).toBe(mixHex('#ffffff', NEAR_BLACK, CARD_MIX))
  })

  it('presetFor finds presets and returns null for custom', () => {
    expect(presetFor('#8a3cff', '#ff8a2a')).toBe('sunset')
    expect(presetFor('#8A3CFF', '#FF8A2A')).toBe('sunset')
    expect(presetFor('#3a3d46', '#16171b')).toBe('plain')
    expect(presetFor('#8a3cff', '#000000')).toBeNull()
  })

  it('on a dark theme: the dark island as before (the raw colours on the edge, glows at 45%)', () => {
    for (const p of THEME_PRESETS) {
      const v = cardVars(p.top, p.bottom)
      expect(v['--card-top']).toBe(mixHex(p.top, NEAR_BLACK, CARD_MIX))
      expect(v['--panel-bottom']).toBe(mixHex(p.bottom, NEAR_BLACK, PANEL_MIX))
      expect([v['--card-text'], v['--card-text-2']]).toEqual([CARD_TEXT, CARD_TEXT_2])
      expect([v['--glow-top'], v['--glow-bottom']]).toEqual([p.top, p.bottom])
      expect([v['--glow-top-a'], v['--glow-bottom-a']]).toEqual([`${p.top}73`, `${p.bottom}73`])
      expect(v['--card-line']).toBe('rgb(242 243 246 / .08)')
    }
  })
})

// #139: on a light theme the card is light too (its two colours mixed into white), with dark text, and light card
// colours (pink + white) still show: the edge is moved just far enough to stand off the app's surfaces.
describe('cardTheme on a light theme (#139)', () => {
  const ground = ['#e9eaed', '#f4f4f6', '#ffffff']
  const light = { light: true, ground }
  const SWEEP = ['#ffffff', '#ffb6d9', '#ffff00', '#00ffff', '#8a3cff', '#3a3d46', '#000000', '#0d1430', '#ff8a2a']

  it('surfaces are the colours mixed into white; panels a little whiter', () => {
    expect([CARD_MIX_LIGHT, PANEL_MIX_LIGHT, NEAR_WHITE]).toEqual([0.4, 0.22, '#ffffff'])
    const v = cardVars('#ffb6d9', '#ffffff', light)
    expect(v['--card-top']).toBe(mixHex('#ffb6d9', NEAR_WHITE, CARD_MIX_LIGHT))
    expect(v['--card-bottom']).toBe('#ffffff')
    expect(v['--panel-top']).toBe(mixHex('#ffb6d9', NEAR_WHITE, PANEL_MIX_LIGHT))
  })

  it('text is dark and reads on every surface, whatever the colours', () => {
    for (const top of SWEEP) for (const bottom of SWEEP) {
      const v = cardVars(top, bottom, light)
      for (const s of [v['--card-top'], v['--card-bottom'], v['--panel-top'], v['--panel-bottom']]) {
        expect(contrast(v['--card-text'], s), `${top} ${bottom} ${s}`).toBeGreaterThanOrEqual(7)
        expect(contrast(v['--card-text-2'], s), `${top} ${bottom} ${s}`).toBeGreaterThanOrEqual(4.5)
        // the voice-room line and errors too (review: the theme's own -text tokens fell under 3:1 on a tinted card)
        expect(contrast(v['--card-ok'], s), `ok ${top} ${bottom} ${s}`).toBeGreaterThanOrEqual(4.5)
        expect(contrast(v['--card-danger'], s), `danger ${top} ${bottom} ${s}`).toBeGreaterThanOrEqual(4.5)
      }
      expect(contrast(v['--card-text'], '#ffffff')).toBeGreaterThan(contrast(v['--card-text'], '#000000'))
    }
  })

  it('light card colours still show: the edge stands off the app surfaces (white turns grey, pink a deeper pink)', () => {
    const v = cardVars('#ffb6d9', '#ffffff', light)
    for (const g of ground) for (const e of [v['--glow-top'], v['--glow-bottom']]) expect(contrast(e, g)).toBeGreaterThanOrEqual(1.5)
    expect(v['--glow-bottom']).not.toBe('#ffffff')
    const [r, g, b] = [1, 3, 5].map((i) => parseInt(v['--glow-top'].slice(i, i + 2), 16))
    expect(r).toBeGreaterThan(g) // still pink
    expect(b).toBeGreaterThan(g)
    expect(v['--glow-bottom-a']).toBe(`${v['--glow-bottom']}73`)
    // a dark colour is already off the ground: kept as picked
    expect(cardVars('#8a3cff', '#ff8a2a', light)['--glow-top']).toBe('#8a3cff')
  })

  it('lines, hovers and the lift follow the light card', () => {
    const v = cardVars('#3a3d46', '#16171b', light)
    expect(v['--card-line']).toBe('rgb(23 24 28 / .1)')
    expect(v['--card-lift']).toBe('var(--shadow-2)')
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
