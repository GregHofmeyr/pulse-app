// @vitest-environment jsdom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import Name from './Name.svelte'
import { app } from '../lib/store.svelte'
import { emptyState } from '../lib/state'
import { profileUi, closeCard } from '../lib/profileCard.svelte'
import { appearance } from '../lib/appearance.svelte'

let cmp: ReturnType<typeof mount> | undefined
const user = (style: { font: string | null; colors: string[] }) => ({ id: 'U2', username: 'sam', avatar_hash: null, avatar_still: null, style })
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
  it('show name styles off renders plain', () => {
    setup({ font: 'fredoka', colors: ['#ffbe7a'] })
    appearance.showNameStyles = false
    const s = show({ interactive: false }).getAttribute('style')!
    expect(s).not.toContain('Fredoka')
    
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
})
