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
})
