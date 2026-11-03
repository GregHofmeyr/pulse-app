// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { deriveAccent, surfacesFor } from './themes'

async function fresh() {
  vi.resetModules()
  return import('./theme.svelte')
}
const root = () => document.documentElement
beforeEach(() => {
  localStorage.clear()
  root().removeAttribute('data-theme')
  root().removeAttribute('data-tint')
  root().removeAttribute('style')
})
afterEach(() => {
  vi.restoreAllMocks()
  delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__
})

describe('theme settings', () => {
  it('defaults: Pulse Dark, plain tint, the theme accent, Normal names', async () => {
    const m = await fresh()
    expect({ ...m.theme }).toEqual({ key: 'dark', tint: 'plain', accent: null, names: 'normal' })
    expect(m.nameRatio()).toBe(4.5)
  })

  it('each choice is remembered on this device', async () => {
    let m = await fresh()
    m.setTheme('light')
    m.setTint('pink')
    m.setAccent('#3DDC97')
    m.setNameReadability('high')
    expect(localStorage.getItem('pulse.theme')).toBe('light')
    expect(localStorage.getItem('pulse.lightTint')).toBe('pink')
    expect(localStorage.getItem('pulse.accent')).toBe('#3ddc97')
    expect(localStorage.getItem('pulse.nameReadability')).toBe('high')
    m = await fresh()
    expect({ ...m.theme }).toEqual({ key: 'light', tint: 'pink', accent: '#3ddc97', names: 'high' })
    expect(m.nameRatio()).toBe(7)
    m.setAccent(null)
    expect(localStorage.getItem('pulse.accent')).toBeNull()
  })

  it('a saved value it does not know falls back to the default', async () => {
    localStorage.setItem('pulse.theme', 'nebula')
    localStorage.setItem('pulse.lightTint', 'teal')
    localStorage.setItem('pulse.accent', 'red')
    localStorage.setItem('pulse.nameReadability', '9')
    const m = await fresh()
    expect({ ...m.theme }).toEqual({ key: 'dark', tint: 'plain', accent: null, names: 'normal' })
  })

  it('survives a throwing localStorage', async () => {
    vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => { throw new Error('denied') })
    vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => { throw new Error('denied') })
    vi.spyOn(Storage.prototype, 'removeItem').mockImplementation(() => { throw new Error('denied') })
    const m = await fresh()
    expect(m.theme.key).toBe('dark')
    expect(() => m.setTheme('oled')).not.toThrow()
    expect(() => m.setAccent(null)).not.toThrow()
    expect(m.theme.key).toBe('oled')
  })
})

describe('applyTheme', () => {
  it('puts the theme (and, on Light only, the tint) on <html>', async () => {
    const m = await fresh()
    m.applyTheme()
    expect(root().dataset.theme).toBe('dark')
    expect(root().dataset.tint).toBeUndefined()
    m.setTint('orange') // remembered, but Pulse Dark has no tint
    expect(root().dataset.tint).toBeUndefined()
    m.setTheme('light')
    expect(root().dataset.theme).toBe('light')
    expect(root().dataset.tint).toBe('orange')
    m.setTheme('pastel')
    expect(root().dataset.tint).toBeUndefined()
  })

  it("a custom accent sets the whole derived family inline for the theme it's on; the theme accent clears it", async () => {
    const m = await fresh()
    m.setTheme('light')
    m.setTint('blue')
    m.setAccent('#fffde0')
    const want = deriveAccent('#fffde0', surfacesFor('light', 'blue'), true)
    for (const [k, v] of Object.entries(want)) expect(root().style.getPropertyValue(k), k).toBe(v)
    m.setTheme('oled') // re-derived for OLED's surfaces
    expect(root().style.getPropertyValue('--accent-text')).toBe(deriveAccent('#fffde0', surfacesFor('oled', 'plain'), false)['--accent-text'])
    m.setAccent(null)
    for (const k of Object.keys(want)) expect(root().style.getPropertyValue(k), k).toBe('')
  })

  it('tells the app window its background colour (inside the app only)', async () => {
    const invoke = vi.fn(() => Promise.resolve())
    const m = await fresh()
    m.setTheme('midnight') // not inside the app: nothing to call, nothing thrown
    ;(window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = { invoke, transformCallback: () => 0 }
    m.setTheme('light')
    m.setTint('green')
    const calls = invoke.mock.calls as unknown as [string, Record<string, unknown>][]
    // an accent change can't change the window colour: no call for it
    m.setAccent('#3ddc97')
    expect(calls.filter((c) => c[0] === 'set_window_background').map((c) => c[1])).toEqual([{ color: '#e9eaed' }, { color: '#e5eee8' }])
  })

  it("names measure against the surface they sit on, in the current theme", async () => {
    const m = await fresh()
    m.setTheme('light')
    expect(m.nameBg('chat')).toBe('#ffffff')
    expect(m.nameBg('side')).toBe('#f4f4f6')
    m.setTheme('dark')
    // a menu (the @-mention list, search suggestions) is lighter than the chat on dark themes
    expect(m.nameBg('menu')).toBe('#2e3037')
    // the profile card is a dark island in every theme
    expect(m.nameBg('island')).toBe(m.ISLAND_BG)
    // …measured against the lightest a preset card gets (Sunset's orange end, mixed into near-black)
    const { THEME_PRESETS, mixHex, NEAR_BLACK, CARD_MIX } = await import('./cardTheme')
    const { luminance, parseHex } = await import('./color')
    for (const p of THEME_PRESETS) for (const c of [p.top, p.bottom]) expect(luminance(parseHex(mixHex(c, NEAR_BLACK, CARD_MIX))!)).toBeLessThanOrEqual(luminance(parseHex(m.ISLAND_BG)!))
  })
})
