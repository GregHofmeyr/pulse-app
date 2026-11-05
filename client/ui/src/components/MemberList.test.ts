// @vitest-environment jsdom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { fakeMatchMedia, FakeIntersectionObserver } from '../lib/testMotion'

const invoke = vi.fn(async (..._a: unknown[]): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...a: unknown[]) => invoke(...a) }))
vi.stubGlobal('IntersectionObserver', FakeIntersectionObserver)
vi.stubGlobal('matchMedia', fakeMatchMedia(false).matchMedia)

import MemberList from './MemberList.svelte'
import { app } from '../lib/store.svelte'
import { emptyState } from '../lib/state'
import { appearance } from '../lib/appearance.svelte'
import { motion } from '../lib/animate.svelte'
import { closeCard, profileUi } from '../lib/profileCard.svelte'

const A = 'a'.repeat(64)
const S = 'b'.repeat(64)
const url = (h: string) => `pulse-media://localhost/${h}`
const alex = { id: 'U1', username: 'alex', display_name: 'alex', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
const sam = { id: 'U2', username: 'sam', display_name: 'sam', avatar_hash: A, avatar_still: S, style: { font: 'fredoka', colors: ['#ffc4a3'] } }

let cmp: ReturnType<typeof mount> | undefined
const onMessage = vi.fn()
beforeEach(() => {
  invoke.mockReset()
  invoke.mockResolvedValue(undefined)
  onMessage.mockReset()
  profileUi.rename = null
  appearance.showNameStyles = true
  motion.streaming = false
  window.dispatchEvent(new Event('focus'))
  closeCard()
  app.state = { ...emptyState(), me: alex, members: { S1: [{ user: alex, nickname: null }, { user: sam, nickname: 'Sammy' }] } } as never
  cmp = mount(MemberList, { target: document.body, props: { serverId: 'S1', onMessage } })
  flushSync()
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
  closeCard()
})
const row = (n: number) => document.querySelectorAll<HTMLElement>('.members .row')[n]

describe('MemberList', () => {
  it('rows use Name and Avatar', () => {
    const r = row(1)
    const name = r.querySelector<HTMLElement>('.name')!
    expect(name.textContent).toBe('Sammy')
    expect(name.getAttribute('style')).toContain('Fredoka')
    expect(name.getAttribute('style')).toContain('rgb(255, 196, 163)')
    const img = r.querySelector('img')!
    FakeIntersectionObserver.set(img, true)
    flushSync()
    expect(img.getAttribute('src')).toBe(url(S))
    name.click()
    flushSync()
    expect(profileUi.card).toMatchObject({ userId: 'U2', serverId: 'S1' })
  })
  it('the avatar animates while the pointer is over its row', () => {
    const r = row(1)
    const img = r.querySelector('img')!
    FakeIntersectionObserver.set(img, true)
    flushSync()
    r.dispatchEvent(new Event('pointerenter'))
    flushSync()
    expect(img.getAttribute('src')).toBe(url(A))
    r.dispatchEvent(new Event('pointerleave'))
    flushSync()
    expect(img.getAttribute('src')).toBe(url(S))
  })
  it('a profile change restyles the row live', () => {
    app.state = { ...app.state, members: { S1: [app.state.members.S1[0], { user: { ...sam, style: { font: null, colors: ['#a3d8ff'] } }, nickname: 'Sammy' }] } }
    flushSync()
    expect(row(1).querySelector('.name')!.getAttribute('style')).toContain('rgb(163, 216, 255)')
  })

  it('shows the name that applies, with the display name small under a nickname, never the username (#47)', () => {
    app.state = { ...app.state, members: { S1: [app.state.members.S1[0], { user: { ...sam, display_name: 'Sam 🎸' }, nickname: 'Sammy' }] } }
    flushSync()
    expect(row(1).querySelector('.name')!.textContent).toBe('Sammy')
    expect(row(1).querySelector('small')!.textContent).toBe('Sam 🎸')
    expect(row(1).textContent).not.toContain('sam')
    // no nickname: the display name, nothing small
    app.state = { ...app.state, members: { S1: [app.state.members.S1[0], { user: { ...sam, display_name: 'Sam 🎸' }, nickname: null }] } }
    flushSync()
    expect(row(1).querySelector('.name')!.textContent).toBe('Sam 🎸')
    expect(row(1).querySelector('small')).toBeNull()
    // your private nickname for them, when they have no server nickname
    app.state = { ...app.state, privateNicks: { U2: 'Bestie' } }
    flushSync()
    expect(row(1).querySelector('.name')!.textContent).toBe('Bestie')
    expect(row(1).querySelector('small')!.textContent).toBe('Sam 🎸')
  })
})

describe('MemberList row menu (#113)', () => {
  const menu = () => document.querySelector<HTMLElement>('[role=menu]')
  const items = () => [...document.querySelectorAll<HTMLElement>('[role=menu] [data-item]')]
  const ids = () => items().map((i) => i.dataset.item)
  const label = (id: string) => document.querySelector(`[role=menu] [data-item="${id}"]`)!.textContent
  const pickItem = (id: string) => {
    document.querySelector<HTMLElement>(`[role=menu] [data-item="${id}"]`)!.click()
    flushSync()
  }
  const rightClick = (el: Element) => {
    el.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: 30, clientY: 40 }))
    flushSync()
  }
  const key = (el: Element, k: string, init: KeyboardEventInit = {}) => {
    el.dispatchEvent(new KeyboardEvent('keydown', { key: k, bubbles: true, cancelable: true, ...init }))
    flushSync()
  }

  it('right-click on a row opens its menu: profile, message, server and private nickname (#137)', () => {
    rightClick(row(1))
    expect(menu()).not.toBeNull()
    expect(menu()!.getAttribute('aria-label')).toBe('Actions for Sammy')
    expect(ids()).toEqual(['profile', 'message', 'nick', 'private-nick'])
    expect(label('private-nick')).toBe('Set private nickname')
    expect(label('profile')).toBe('View profile')
    expect(label('message')).toBe('Message')
    expect(label('nick')).toBe('Change server nickname')
    expect(document.activeElement).toBe(items()[0])
  })

  it('no reset items; with a private nickname set it says Change (#137)', () => {
    app.state = { ...app.state, privateNicks: { U2: 'Bestie' } }
    flushSync()
    rightClick(row(1))
    expect(ids()).toEqual(['profile', 'message', 'nick', 'private-nick'])
    expect(label('private-nick')).toBe('Change private nickname')
  })

  it('your own row: profile and your server nickname, no message', () => {
    rightClick(row(0))
    expect(ids()).toEqual(['profile', 'nick'])
  })

  it('each item does what it says', () => {
    rightClick(row(1))
    pickItem('nick')
    expect(profileUi.rename).toEqual({ kind: 'server', userId: 'U2', serverId: 'S1' })
    expect(menu()).toBeNull()

    rightClick(row(1))
    pickItem('private-nick')
    expect(profileUi.rename).toEqual({ kind: 'private', userId: 'U2' })
    expect(menu()).toBeNull()

    rightClick(row(1))
    pickItem('message')
    expect(onMessage).toHaveBeenCalledWith('U2')
    expect(menu()).toBeNull()

    rightClick(row(1))
    pickItem('profile')
    expect(profileUi.card).toMatchObject({ userId: 'U2', serverId: 'S1' })
    expect(menu()).toBeNull()
  })

  it('the keyboard: ContextMenu or Shift+F10 on the name opens it, arrows wrap, Esc and Tab close and return focus', () => {
    const name = row(1).querySelector<HTMLElement>('.name')!
    name.focus()
    key(name, 'F10', { shiftKey: true })
    expect(menu()).not.toBeNull()
    key(document.activeElement!, 'Escape')
    expect(menu()).toBeNull()
    expect(document.activeElement).toBe(name)
    key(name, 'ContextMenu')
    const list = items()
    expect(document.activeElement).toBe(list[0])
    key(document.activeElement!, 'ArrowUp')
    expect(document.activeElement).toBe(list[list.length - 1])
    key(document.activeElement!, 'ArrowDown')
    expect(document.activeElement).toBe(list[0])
    key(document.activeElement!, 'ArrowDown')
    expect(document.activeElement).toBe(list[1])
    key(document.activeElement!, 'End')
    expect(document.activeElement).toBe(list[list.length - 1])
    key(document.activeElement!, 'Home')
    expect(document.activeElement).toBe(list[0])
    key(document.activeElement!, 'Tab')
    expect(menu()).toBeNull()
    expect(document.activeElement).toBe(name)
  })

  it('Enter picks the focused item once', () => {
    const name = row(1).querySelector<HTMLElement>('.name')!
    name.focus()
    key(name, 'ContextMenu')
    key(document.activeElement!, 'ArrowDown')
    key(document.activeElement!, 'Enter')
    expect(onMessage).toHaveBeenCalledTimes(1)
    expect(menu()).toBeNull()
  })

  it('a click outside closes it', async () => {
    rightClick(row(1))
    await new Promise((r) => setTimeout(r, 0))
    document.body.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true }))
    flushSync()
    expect(menu()).toBeNull()
  })
})
