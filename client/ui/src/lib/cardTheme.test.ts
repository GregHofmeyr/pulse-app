import { describe, expect, it } from 'vitest'
import { CARD_MIX, CARD_TEXT, CARD_TEXT_2, cardVars, DEFAULT_THEME, mixHex, NEAR_BLACK, PANEL_MIX, presetFor, THEME_PRESETS } from './cardTheme'
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
    expect(Object.keys(v).sort()).toEqual(['--card-bottom', '--card-text', '--card-text-2', '--card-top', '--glow-bottom', '--glow-top', '--panel-bottom', '--panel-top'])
    expect(v['--glow-top']).toBe('#ffffff')
    expect(v['--card-top']).toBe(mixHex('#ffffff', NEAR_BLACK, CARD_MIX))
  })

  it('presetFor finds presets and returns null for custom', () => {
    expect(presetFor('#8a3cff', '#ff8a2a')).toBe('sunset')
    expect(presetFor('#8A3CFF', '#FF8A2A')).toBe('sunset')
    expect(presetFor('#3a3d46', '#16171b')).toBe('plain')
    expect(presetFor('#8a3cff', '#000000')).toBeNull()
  })
})
