// @vitest-environment jsdom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import AppearanceSettings from './AppearanceSettings.svelte'
import Name from './Name.svelte'
import { app } from '../lib/store.svelte'
import { emptyState } from '../lib/state'
import { appearance, setShowNameStyles } from '../lib/appearance.svelte'
import { setAccent, setNameReadability, setTheme, setTint, theme } from '../lib/theme.svelte'
import { THEMES } from '../lib/themes'

const cmps: ReturnType<typeof mount>[] = []
beforeEach(() => {
  localStorage.clear()
  setShowNameStyles(true)
  app.state = { ...emptyState(), members: { S1: [{ user: { id: 'U2', username: 'sam', display_name: 'sam', avatar_hash: null, avatar_still: null, style: { font: 'fredoka', colors: ['#ffc4a3'] } }, nickname: null }] } } as never
})
afterEach(() => {
  cmps.splice(0).forEach((c) => unmount(c))
  document.body.innerHTML = ''
  localStorage.clear()
})

describe('AppearanceSettings', () => {
  it('the switch turns name styles off everywhere', () => {
    cmps.push(mount(Name, { target: document.body, props: { userId: 'U2', serverId: 'S1', interactive: false } }))
    cmps.push(mount(AppearanceSettings, { target: document.body }))
    flushSync()
    const name = document.querySelector<HTMLElement>('.name')!
    expect(name.getAttribute('style')).toContain('Fredoka')
    const sw = document.querySelector<HTMLInputElement>('input[type=checkbox][role=switch]')!
    expect(sw.checked).toBe(true)
    expect(document.body.textContent).toContain('Show name styles')
    expect(document.body.textContent).toContain("Show everyone's chosen name fonts and colours. Off shows plain names (role colours still show).")
    sw.click()
    flushSync()
    expect(appearance.showNameStyles).toBe(false)
    expect(localStorage.getItem('pulse.showNameStyles')).toBe('false')
    expect(name.getAttribute('style')).not.toContain('Fredoka')
    expect(name.getAttribute('style')).not.toContain('color')
  })
})

describe('AppearanceSettings: theme, accent and name readability (#42)', () => {
  const html = document.documentElement
  beforeEach(() => {
    setTheme('dark')
    setTint('plain')
    setAccent(null)
    setNameReadability('normal')
  })
  const open = () => {
    cmps.push(mount(AppearanceSettings, { target: document.body }))
    flushSync()
  }
  const button = (group: string, label: string) => {
    const g = document.querySelector<HTMLElement>(`[role=group][aria-label="${group}"]`)!
    return [...g.querySelectorAll<HTMLButtonElement>('button')].find((b) => b.textContent!.trim() === label || b.getAttribute('aria-label') === label)!
  }

  it('lists every theme as a live preview; picking one switches the app and is remembered', () => {
    open()
    const cards = document.querySelectorAll<HTMLButtonElement>('[role=group][aria-label="Theme"] button')
    expect([...cards].map((c) => c.textContent!.trim())).toEqual(THEMES.map((t) => t.label))
    // each preview carries its own theme, so it shows that theme's colours whatever the app is on
    expect([...cards].map((c) => c.querySelector<HTMLElement>('.preview')!.dataset.theme)).toEqual(THEMES.map((t) => t.key))
    expect(button('Theme', 'Pulse Dark').getAttribute('aria-pressed')).toBe('true')
    button('Theme', 'OLED Black').click()
    flushSync()
    expect(html.dataset.theme).toBe('oled')
    expect(localStorage.getItem('pulse.theme')).toBe('oled')
    expect(button('Theme', 'OLED Black').getAttribute('aria-pressed')).toBe('true')
    expect(button('Theme', 'Pulse Dark').getAttribute('aria-pressed')).toBe('false')
  })

  it("Light's tint picker shows on Light only, and its preview follows the tint", () => {
    open()
    expect(document.querySelector('[role=group][aria-label="Tint"]')).toBeNull()
    button('Theme', 'Light').click()
    flushSync()
    expect(document.querySelectorAll('[role=group][aria-label="Tint"] button')).toHaveLength(5)
    button('Tint', 'Pink').click()
    flushSync()
    expect(html.dataset.tint).toBe('pink')
    expect(localStorage.getItem('pulse.lightTint')).toBe('pink')
    expect(button('Tint', 'Pink').getAttribute('aria-pressed')).toBe('true')
    expect(button('Theme', 'Light').querySelector<HTMLElement>('.preview')!.dataset.tint).toBe('pink')
  })

  it('accent: the theme default, a preset, or any colour', () => {
    open()
    expect(button('Accent', 'Theme default').getAttribute('aria-pressed')).toBe('true')
    button('Accent', 'Mint').click()
    flushSync()
    expect(html.style.getPropertyValue('--accent')).toBe('#3ddc97')
    expect(localStorage.getItem('pulse.accent')).toBe('#3ddc97')
    expect(button('Accent', 'Mint').getAttribute('aria-pressed')).toBe('true')
    const pick = document.querySelector<HTMLInputElement>('input[type=color]')!
    expect(pick.value).toBe('#3ddc97')
    pick.value = '#123456'
    pick.dispatchEvent(new Event('input', { bubbles: true }))
    flushSync()
    expect(html.style.getPropertyValue('--accent')).toBe('#123456')
    expect(html.style.getPropertyValue('--on-accent')).toBe('#ffffff')
    expect(button('Accent', 'Mint').getAttribute('aria-pressed')).toBe('false')
    button('Accent', 'Theme default').click()
    flushSync()
    expect(html.style.getPropertyValue('--accent')).toBe('')
    expect(localStorage.getItem('pulse.accent')).toBeNull()
  })

  it('name readability: a three-step slider with a live sample of name styles', () => {
    open()
    const slider = document.querySelector<HTMLInputElement>('input[type=range]')!
    expect(slider.value).toBe('1')
    expect(slider.getAttribute('aria-valuetext')).toBe('Normal (4.5:1)')
    const sample = document.querySelectorAll<HTMLElement>('.sample .name')
    expect(sample.length).toBeGreaterThanOrEqual(4)
    button('Theme', 'Light').click()
    flushSync()
    const styles = () => [...sample].map((n) => n.getAttribute('style'))
    const normal = styles()
    slider.value = '2'
    slider.dispatchEvent(new Event('input', { bubbles: true }))
    flushSync()
    expect(theme.names).toBe('high')
    expect(localStorage.getItem('pulse.nameReadability')).toBe('high')
    expect(slider.getAttribute('aria-valuetext')).toBe('High (7:1)')
    expect(styles()).not.toEqual(normal)
  })
})
