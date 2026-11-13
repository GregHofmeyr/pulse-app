// @vitest-environment jsdom
import { flushSync, mount, unmount } from 'svelte'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { compile } from 'svelte/compiler'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import Name from './Name.svelte'
import { app } from '../lib/store.svelte'
import { emptyState } from '../lib/state'
import { profileUi, closeCard } from '../lib/profileCard.svelte'
import { appearance } from '../lib/appearance.svelte'
import { setNameReadability, setTheme } from '../lib/theme.svelte'
import { surfacesFor } from '../lib/themes'
import { contrast, toHex } from '../lib/color'

let cmp: ReturnType<typeof mount> | undefined
const user = (style: { font: string | null; colors: string[] }) => ({ id: 'U2', username: 'sam', display_name: 'sam', avatar_hash: null, avatar_still: null, style })
function setup(style: { font: string | null; colors: string[] }, roleColor?: string) {
  app.state = {
    ...emptyState(),
    members: { S1: [{ user: user(style), nickname: null }] },
    roleColors: roleColor ? { S1: { U2: roleColor } } : {},
  } as never
}
function show(props: Record<string, unknown> = {}) {
  cmp = mount(Name, { target: document.body, props: { userId: 'U2', serverId: 'S1', ...props } })
  flushSync()
  return document.querySelector('.name') as HTMLElement
}

beforeEach(() => {
  appearance.showNameStyles = true
  setTheme('dark')
  setNameReadability('normal')
  closeCard()
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})

describe('Name', () => {
  it('default style is plain text', () => {
    setup({ font: null, colors: [] })
    const el = show({ interactive: false })
    expect(el.textContent).toBe('sam')
    expect(el.getAttribute('style')).not.toMatch(/color|font-family|gradient/)
  })
  it('font and gradient applied', () => {
    setup({ font: 'fredoka', colors: ['#ffbe7a', '#ff7fae'] })
    const s = show({ interactive: false }).getAttribute('style')!
    expect(s).toContain('Fredoka')
    expect(s).toContain('linear-gradient(90deg, rgb(255, 190, 122), rgb(255, 127, 174))')
  })
  it('role colour wins over the personal colour', () => {
    setup({ font: 'fredoka', colors: ['#ffbe7a', '#ff7fae'] }, '#a3d8ff')
    const s = show({ interactive: false }).getAttribute('style')!
    expect(s).toContain('Fredoka')
    expect(s).toContain('color: rgb(163, 216, 255)')
    expect(s).not.toContain('gradient')
  })
  it('readable where it sits (#42): darkened on Light, by the chosen level, against the chat or the sidebar', () => {
    const rgb = (s: string) => toHex(/color: rgb\((\d+), (\d+), (\d+)\)/.exec(s)!.slice(1).map(Number) as [number, number, number])
    setup({ font: null, colors: ['#ffc4a3'] })
    setTheme('light')
    const chat = show({ interactive: false })
    const onChat = rgb(chat.getAttribute('style')!)
    expect(contrast(onChat, '#ffffff')).toBeGreaterThanOrEqual(4.5)
    expect(contrast(onChat, '#ffffff')).toBeLessThan(5)
    setNameReadability('high')
    flushSync()
    expect(contrast(rgb(chat.getAttribute('style')!), '#ffffff')).toBeGreaterThanOrEqual(7)
    unmount(cmp!)
    document.body.innerHTML = ''
    const side = show({ interactive: false, surface: 'side' })
    expect(contrast(rgb(side.getAttribute('style')!), surfacesFor('light', 'plain').bg1)).toBeGreaterThanOrEqual(7)
    unmount(cmp!)
    document.body.innerHTML = ''
    // on a profile card it is measured against the card's own colour, whatever the theme (#156): a dark card keeps it
    // light, a light card darkens it
    const onDark = show({ interactive: false, bg: '#22325e' })
    expect(rgb(onDark.getAttribute('style')!)).toBe('#ffc4a3')
    unmount(cmp!)
    document.body.innerHTML = ''
    setTheme('dark')
    const onLight = show({ interactive: false, bg: '#ffc8e0' })
    expect(contrast(rgb(onLight.getAttribute('style')!), '#ffc8e0')).toBeGreaterThanOrEqual(4.5)
  })
  it('restyles live when the theme changes, and measures a menu name against the menu', () => {
    setup({ font: null, colors: [] }, '#3b4bd8')
    const rgb = (s: string) => toHex(/color: rgb\((\d+), (\d+), (\d+)\)/.exec(s)!.slice(1).map(Number) as [number, number, number])
    const name = show({ interactive: false })
    const dark = rgb(name.getAttribute('style')!)
    setTheme('light')
    flushSync()
    expect(rgb(name.getAttribute('style')!)).not.toBe(dark)
    expect(contrast(rgb(name.getAttribute('style')!), '#ffffff')).toBeGreaterThanOrEqual(4.5)
    setTheme('dark')
    flushSync()
    expect(rgb(name.getAttribute('style')!)).toBe(dark)
    unmount(cmp!)
    document.body.innerHTML = ''
    const menu = show({ interactive: false, surface: 'menu' })
    expect(contrast(rgb(menu.getAttribute('style')!), surfacesFor('dark', 'plain').menu)).toBeGreaterThanOrEqual(4.5)
  })
  it('show name styles off renders plain', () => {
    setup({ font: 'fredoka', colors: ['#ffbe7a'] })
    appearance.showNameStyles = false
    const s = show({ interactive: false }).getAttribute('style') ?? ''
    expect(s).not.toContain('Fredoka')
    expect(s).not.toMatch(/color|gradient/)
    unmount(cmp!)
    cmp = undefined
    // a role colour still shows
    setup({ font: 'fredoka', colors: ['#ffbe7a'] }, '#a3d8ff')
    const r = show({ interactive: false }).getAttribute('style')!
    expect(r).toContain('color: rgb(163, 216, 255)')
    expect(r).not.toContain('Fredoka')
  })
  it('restyles live when the profile changes', () => {
    setup({ font: null, colors: [] })
    const el = show({ interactive: false })
    app.state = { ...app.state, members: { S1: [{ user: user({ font: null, colors: ['#ffc4a3'] }), nickname: null }] } }
    flushSync()
    expect(el.getAttribute('style')).toContain('rgb(255, 196, 163)')
  })
  it('clicking opens the card for that server', () => {
    setup({ font: null, colors: [] })
    const el = show()
    expect(el.tagName).toBe('BUTTON')
    el.click()
    flushSync()
    expect(profileUi.card).toMatchObject({ userId: 'U2', serverId: 'S1' })
  })
  it('non-interactive renders a span', () => {
    setup({ font: null, colors: [] })
    expect(show({ interactive: false, text: 'Sammy' }).tagName).toBe('SPAN')
    expect(document.querySelector('.name')!.textContent).toBe('Sammy')
  })

  it('hugs its text even in a stretching parent, so a gradient spans the name itself', () => {
    // A flex column (the card, the editor preview, the account panel) stretches its children: a
    // stretched name paints its gradient across the whole row and shows only the first colour.
    setup({ font: null, colors: ['#ff0000', '#0000ff'] })
    const el = show({ interactive: false, text: 'Sammy' })
    const hash = [...el.classList].find((c) => c.startsWith('svelte-'))!
    const src = readFileSync(resolve(__dirname, 'Name.svelte'), 'utf8')
    const style = document.createElement('style')
    style.textContent = compile(src, { filename: 'Name.svelte', css: 'external', cssHash: () => hash }).css!.code
    document.head.append(style)
    expect(getComputedStyle(el).width).toBe('fit-content')
    style.remove()
  })
})
