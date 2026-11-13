// @vitest-environment jsdom
// "Your people" (#45): the split, the order, the dots and the line under each name all come from presenceOf.
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { fakeMatchMedia, FakeIntersectionObserver } from '../lib/testMotion'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => undefined) }))
vi.stubGlobal('IntersectionObserver', FakeIntersectionObserver)
vi.stubGlobal('matchMedia', fakeMatchMedia(false).matchMedia)

import PeopleBoard from './PeopleBoard.svelte'
import { app, clock, resetState } from '../lib/store.svelte'
import { emptyState } from '../lib/state'
import { invoke } from '@tauri-apps/api/core'

const user = (id: string, name: string) => ({ id, username: name, display_name: name, avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } })
let cmp: ReturnType<typeof mount> | undefined

beforeEach(() => {
  resetState()
  clock.nowIso = '2026-11-05T10:00:00.000Z'
  app.state = {
    ...emptyState(), me: user('U1', 'alex'),
    people: {
      U1: { user: user('U1', 'alex'), online: true, last_seen_at: null, status: 'online' },
      U2: { user: user('U2', 'sam'), online: true, last_seen_at: null, status: 'online' },
      U3: { user: user('U3', 'kim'), online: true, last_seen_at: null, status: 'dnd' },
      // appearing offline to you (or invisible): offline, last seen frozen
      U4: { user: user('U4', 'jo'), online: false, last_seen_at: '2026-11-05T07:00:00.000Z', status: 'offline' },
    },
  }
  cmp = mount(PeopleBoard, { target: document.body, props: { onOpen: () => {}, onJoinVoice: () => {} } })
  flushSync()
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})

describe('PeopleBoard', () => {
  it('Do Not Disturb counts as online, with its own dot and line; offline shows last seen', () => {
    expect([...document.querySelectorAll('.lbl')].map((l) => l.textContent)).toEqual(['ONLINE — 2', 'OFFLINE — 1'])
    const card = (name: string) => [...document.querySelectorAll<HTMLElement>('.card')].find((c) => c.textContent?.includes(name))!
    expect(card('kim').querySelector('.pres')!.classList.contains('dnd')).toBe(true)
    expect(card('kim').querySelector('.status')!.textContent?.trim()).toBe('do not disturb')
    expect(card('sam').querySelector('.status')!.textContent?.trim()).toBe('online')
    expect(card('jo').querySelector('.pres')!.classList.contains('offline')).toBe(true)
    expect(card('jo').querySelector('.status')!.textContent?.trim()).toBe('last seen 3h ago')
  })

  it('Call (#50) is enabled: it opens your DM with them and starts its call', async () => {
    const dm = { id: 'DM2', server_id: null, kind: 'dm' as const, name: null, position: 0 }
    vi.mocked(invoke).mockImplementation(async (c) => (c === 'create_dm' ? dm : undefined) as never)
    const card = [...document.querySelectorAll<HTMLElement>('.card')].find((c) => c.textContent?.includes('sam'))!
    const btn = [...card.querySelectorAll<HTMLButtonElement>('button')].find((b) => b.textContent?.includes('Call'))!
    expect(btn.disabled).toBe(false)
    btn.click()
    await vi.waitFor(() => expect(vi.mocked(invoke).mock.calls.some(([c]) => c === 'join_voice')).toBe(true))
    const names = vi.mocked(invoke).mock.calls.map(([c]) => c)
    expect(names.indexOf('create_dm')).toBeLessThan(names.indexOf('start_call'))
    expect(names.indexOf('start_call')).toBeLessThan(names.indexOf('join_voice'))
    expect(vi.mocked(invoke).mock.calls.find(([c]) => c === 'start_call')![1]).toEqual({ channelId: 'DM2' })
    vi.mocked(invoke).mockImplementation(async () => undefined)
  })
})

describe('PeopleBoard person menu (#135)', () => {
  const card = (name: string) => [...document.querySelectorAll<HTMLElement>('.card')].find((c) => c.textContent?.includes(name))!
  const menu = () => document.querySelector<HTMLElement>('[role=menu]')
  const ids = () => [...document.querySelectorAll<HTMLElement>('[role=menu] [data-item]')].map((i) => i.dataset.item)
  const key = (el: Element, k: string, init: KeyboardEventInit = {}) => {
    el.dispatchEvent(new KeyboardEvent('keydown', { key: k, bubbles: true, cancelable: true, ...init }))
    flushSync()
  }

  it('right-click a card: their person menu, outside servers', () => {
    const e = new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: 10, clientY: 10 })
    card('sam').dispatchEvent(e)
    flushSync()
    expect(e.defaultPrevented).toBe(true)
    expect(menu()!.getAttribute('aria-label')).toBe('Actions for sam')
    expect(ids()).toEqual(['profile', 'message', 'private-nick', 'live-notify', 'appear-offline'])
  })

  it('Shift+F10 on the name opens it; Esc gives focus back', () => {
    const name = card('kim').querySelector<HTMLElement>('.name')!
    name.focus()
    key(name, 'F10', { shiftKey: true })
    expect(menu()!.getAttribute('aria-label')).toBe('Actions for kim')
    key(document.activeElement!, 'Escape')
    expect(menu()).toBeNull()
    expect(document.activeElement).toBe(name)
  })
})
