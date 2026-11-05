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

describe('MemberList status dots (#169)', () => {
  const people = (samStatus: string, samOnline: boolean) =>
    ({ U1: { user: alex, online: true, last_seen_at: null, status: 'online' }, U2: { user: sam, online: samOnline, last_seen_at: null, status: samStatus } }) as never
  const dot = (n: number) => row(n).querySelector('.av .pres')

  it('each avatar has its person\'s dot, from presenceOf', () => {
    app.state = { ...app.state, people: people('dnd', true) }
    flushSync()
    expect(dot(0)!.classList.contains('online')).toBe(true)
    expect(dot(1)!.classList.contains('dnd')).toBe(true)
    expect(dot(1)!.getAttribute('aria-label')).toBe('Do Not Disturb')
    app.state = { ...app.state, people: people('offline', false) }
    flushSync()
    expect(dot(1)!.classList.contains('offline')).toBe(true)
  })

  it('your own dot follows your status (Invisible is yours alone)', () => {
    app.state = { ...app.state, people: people('online', true), myStatus: { status: 'invisible', dnd_until: null } }
    flushSync()
    // you're under Offline then (#170), where everyone else sees you
    const mine = [...document.querySelectorAll<HTMLElement>('.members .row')].find((r) => r.querySelector('.name')!.textContent === 'alex')!
    expect(mine.querySelector('.pres')!.getAttribute('aria-label')).toBe('Invisible')
  })

  it('never draws someone else as Invisible, whatever the server says (#171)', () => {
    app.state = { ...app.state, people: people('invisible', false) }
    flushSync()
    const samRow = [...document.querySelectorAll<HTMLElement>('.members .row')].find((r) => r.textContent?.includes('Sammy'))!
    expect(samRow.querySelector('.pres')!.classList.contains('offline')).toBe(true)
    expect(document.querySelector('.members .pres.invisible')).toBeNull()
  })
})

describe('MemberList Online and Offline sections (#170)', () => {
  const kim = { ...alex, id: 'U3', username: 'kim', display_name: 'kim' }
  const jo = { ...alex, id: 'U4', username: 'jo', display_name: 'jo' }
  const p = (user: object, status: string) => ({ user, online: status !== 'offline', last_seen_at: null, status })
  const labels = () => [...document.querySelectorAll('.members .label')].map((l) => l.textContent?.trim())
  const section = (name: string) => [...document.querySelectorAll<HTMLElement>('.members .row')].filter((r) => r.dataset.section === name).map((r) => r.querySelector('.name')!.textContent)

  it('Online (DND included) above Offline, each with its count; offline rows dimmed', () => {
    app.state = {
      ...app.state,
      members: { S1: [{ user: jo, nickname: null }, { user: alex, nickname: null }, { user: sam, nickname: 'Sammy' }, { user: kim, nickname: null }] },
      people: { U1: p(alex, 'online'), U2: p(sam, 'dnd'), U3: p(kim, 'offline'), U4: p(jo, 'offline') },
    } as never
    flushSync()
    expect(labels()).toEqual(['Online — 2', 'Offline — 2'])
    expect(section('online')).toEqual(['alex', 'Sammy'])
    expect(section('offline')).toEqual(['jo', 'kim'])
    // the headings sit above their rows
    const all = [...document.querySelectorAll('.members .label, .members .row')].map((e) => e.classList.contains('label') ? e.textContent?.trim() : e.querySelector('.name')!.textContent)
    expect(all).toEqual(['Online — 2', 'alex', 'Sammy', 'Offline — 2', 'jo', 'kim'])
    for (const r of document.querySelectorAll<HTMLElement>('.members .row')) expect(r.classList.contains('off')).toBe(r.dataset.section === 'offline')
  })

  it('someone sent as offline (Invisible, or hiding from you) is under Offline; a section with nobody has no heading', () => {
    app.state = { ...app.state, people: { U1: p(alex, 'online'), U2: p(sam, 'invisible') } } as never
    flushSync()
    expect(labels()).toEqual(['Online — 1', 'Offline — 1'])
    expect(section('offline')).toEqual(['Sammy'])
    app.state = { ...app.state, people: { U1: p(alex, 'online'), U2: p(sam, 'online') } } as never
    flushSync()
    expect(labels()).toEqual(['Online — 2'])
  })

  it('a status change moves them live', () => {
    app.state = { ...app.state, people: { U1: p(alex, 'online'), U2: p(sam, 'online') } } as never
    flushSync()
    expect(section('online')).toEqual(['alex', 'Sammy'])
    app.state = { ...app.state, people: { ...app.state.people, U2: p(sam, 'offline') } } as never
    flushSync()
    expect(section('offline')).toEqual(['Sammy'])
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
    expect(ids()).toEqual(['profile', 'message', 'nick', 'private-nick', 'live-notify', 'appear-offline'])
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
    expect(ids()).toEqual(['profile', 'message', 'nick', 'private-nick', 'live-notify', 'appear-offline'])
    expect(label('private-nick')).toBe('Change private nickname')
  })

  it('Notify me when they go live, and back (#60)', () => {
    rightClick(row(1))
    expect(label('live-notify')).toBe('Notify me when they go live')
    pickItem('live-notify')
    expect(invoke.mock.calls.filter(([c]) => c === 'set_live_notify').map(([, a]) => a)).toEqual([{ userId: 'U2', on: true }])
    expect(menu()).toBeNull()
    app.state = { ...app.state, liveNotify: ['U2'] }
    flushSync()
    rightClick(row(1))
    expect(label('live-notify')).toBe('Stop notifying me when they go live')
    pickItem('live-notify')
    expect(invoke.mock.calls.filter(([c]) => c === 'set_live_notify').at(-1)![1]).toEqual({ userId: 'U2', on: false })
  })

  it('Appear offline to them, and back (#45)', () => {
    rightClick(row(1))
    expect(label('appear-offline')).toBe('Appear offline to them')
    pickItem('appear-offline')
    expect(invoke.mock.calls.filter(([c]) => c === 'set_appear_offline').map(([, a]) => a)).toEqual([{ userId: 'U2', hidden: true }])
    expect(menu()).toBeNull()
    app.state = { ...app.state, appearOfflineTo: ['U2'] }
    flushSync()
    rightClick(row(1))
    expect(label('appear-offline')).toBe('Stop appearing offline to them')
    pickItem('appear-offline')
    expect(invoke.mock.calls.filter(([c]) => c === 'set_appear_offline').at(-1)![1]).toEqual({ userId: 'U2', hidden: false })
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
