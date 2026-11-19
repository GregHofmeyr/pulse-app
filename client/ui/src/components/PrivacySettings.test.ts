// @vitest-environment jsdom
// Settings → Privacy → Appear offline to (#45): everyone you're hidden from, each with Remove.
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { fakeMatchMedia, FakeIntersectionObserver } from '../lib/testMotion'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))
vi.stubGlobal('IntersectionObserver', FakeIntersectionObserver)
vi.stubGlobal('matchMedia', fakeMatchMedia(false).matchMedia)

import PrivacySettings from './PrivacySettings.svelte'
import Settings from './Settings.svelte'
import { app, resetState } from '../lib/store.svelte'
import { applyEvent, emptyState } from '../lib/state'
import { activityPrefs, mediaActivity } from '../lib/activityPrefs.svelte'

const user = (id: string, name: string) => ({ id, username: name, display_name: name, avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } })
const alex = user('U1', 'alex')
let cmp: ReturnType<typeof mount> | undefined

beforeEach(() => {
  resetState()
  invoke.mockReset()
  invoke.mockResolvedValue(undefined)
  app.state = {
    ...emptyState(), me: alex,
    people: {
      U1: { user: alex, online: true, last_seen_at: null, status: 'online' },
      U2: { user: user('U2', 'sam'), online: true, last_seen_at: null, status: 'online' },
      U3: { user: user('U3', 'kim'), online: false, last_seen_at: null, status: 'offline' },
    },
    privateNicks: { U3: 'Kimmy' },
    appearOfflineTo: ['U3', 'U2'],
  }
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})
const rows = () => [...document.querySelectorAll<HTMLElement>('.hidden-list li')]

describe('PrivacySettings → Activity (#275)', () => {
  const switches = () => [...document.querySelectorAll<HTMLInputElement>('input[role="switch"]')]
  const calls = () => invoke.mock.calls.filter(([c]) => c === 'set_media_activity').map(([, a]) => a)

  it('both off until turned on; Show waits for Read; each change tells the core and is kept', () => {
    localStorage.clear()
    activityPrefs.read = false
    activityPrefs.show = false
    cmp = mount(PrivacySettings, { target: document.body })
    flushSync()
    const [read, show, players, browsers] = switches()
    expect([read.checked, show.checked, show.disabled]).toEqual([false, false, true])
    // other players (#322): on until turned off, waiting for Read
    expect([players.checked, players.disabled, browsers.checked, browsers.disabled]).toEqual([true, true, true, true])
    expect(document.querySelector('.reading')).toBeNull()
    read.click()
    flushSync()
    expect(show.disabled).toBe(false)
    const both = { players: true, browsers: true }
    expect(calls()).toEqual([{ read: true, show: false, sources: both }])
    expect(localStorage.getItem('pulse.activity.read')).toBe('true')
    expect(document.querySelector('.reading')!.textContent).toBe('Nothing playing right now.')
    show.click()
    flushSync()
    expect(calls().at(-1)).toEqual({ read: true, show: true, sources: both })
    mediaActivity.playing = { title: 'Song', artist: 'The Band', album: null, spotify_id: null, player: 'spotify' }
    flushSync()
    expect(document.querySelector('.reading')!.textContent).toBe('Spotify is playing Song · The Band')
    mediaActivity.playing = { title: 'Song', artist: 'Jo', album: 'Album', spotify_id: null, player: 'browser' }
    flushSync()
    expect(document.querySelector('.reading')!.textContent).toBe('A browser is playing Song · Jo')
    expect(players.disabled || browsers.disabled).toBe(false)
    browsers.click()
    flushSync()
    expect(calls().at(-1)).toEqual({ read: true, show: true, sources: { players: true, browsers: false } })
    expect(localStorage.getItem('pulse.activity.browsers')).toBe('false')
    players.click()
    flushSync()
    expect(calls().at(-1)).toEqual({ read: true, show: true, sources: { players: false, browsers: false } })
    players.click()
    browsers.click()
    flushSync()
    read.click()
    flushSync()
    expect(calls().at(-1)).toEqual({ read: false, show: true, sources: both })
    expect(show.disabled).toBe(true)
    mediaActivity.playing = null
  })
})

describe('PrivacySettings', () => {
  it('lists everyone you appear offline to, by the name you see them by, with their avatar', () => {
    cmp = mount(PrivacySettings, { target: document.body })
    flushSync()
    expect([...document.querySelectorAll('h3')].map((h) => h.textContent)).toEqual(['Activity', 'Game activity', 'Appear offline to', 'Blocked people'])
    expect(rows().map((r) => r.querySelector('.who')!.textContent?.trim())).toEqual(['Kimmy', 'sam'])
    expect(rows().every((r) => r.querySelector('.av'))).toBe(true)
  })

  it('Remove stops appearing offline to them; the list follows the event', () => {
    cmp = mount(PrivacySettings, { target: document.body })
    flushSync()
    const remove = rows()[1].querySelector<HTMLButtonElement>('button')!
    expect(remove.textContent?.trim()).toBe('Remove')
    expect(remove.getAttribute('aria-label')).toBe('Stop appearing offline to sam')
    remove.click()
    flushSync()
    expect(invoke.mock.calls.filter(([c]) => c === 'set_appear_offline').map(([, a]) => a)).toEqual([{ userId: 'U2', hidden: false }])
    app.state = applyEvent(app.state, { t: 'AppearOfflineChanged', d: { user_id: 'U1', target_id: 'U2', hidden: false } }, 0)
    flushSync()
    expect(rows()).toHaveLength(1)
  })

  it('a failure says why', async () => {
    invoke.mockRejectedValue('no connection')
    cmp = mount(PrivacySettings, { target: document.body })
    flushSync()
    rows()[0].querySelector<HTMLButtonElement>('button')!.click()
    await vi.waitFor(() => expect(document.querySelector('[role=alert]')?.textContent).toContain('no connection'))
  })

  it('nobody: says so', () => {
    app.state = { ...app.state, appearOfflineTo: [] }
    cmp = mount(PrivacySettings, { target: document.body })
    flushSync()
    expect(rows()).toEqual([])
    // (Game activity's "Hide what I'm playing from" list, #436, has its own)
    expect([...document.querySelectorAll('.empty')].find((e) => !e.closest('[data-hidden-games]'))!.textContent).toContain("You don't appear offline to anyone")
  })

  describe('Add person (#188)', () => {
    const jo = user('U4', 'jo')
    const addBtn = () => [...document.querySelectorAll<HTMLButtonElement>('button')].find((b) => b.textContent?.trim() === 'Add person' && !b.closest('[data-hidden-games]'))
    const picker = () => document.querySelector<HTMLElement>('[role=dialog][aria-label="Appear offline to"]')
    const offered = () => [...document.querySelectorAll<HTMLElement>('[role=dialog] .pick')].map((b) => b.querySelector('.nm')!.textContent?.trim())
    const setup = (over: object) => {
      app.state = {
        ...app.state,
        people: { ...app.state.people, U4: { user: jo, online: true, last_seen_at: null, status: 'online' } },
        appearOfflineTo: ['U3'],
        ...over,
      } as never
      cmp = mount(PrivacySettings, { target: document.body })
      flushSync()
    }

    it('opens the people picker with anyone sharing a DM or group with you, never yourself or anyone already hidden', () => {
      // sam shares a DM, kim (hidden) a group, jo nothing
      setup({ dmMembers: { D1: ['U1', 'U2'], G1: ['U1', 'U3'] } })
      expect(picker()).toBeNull()
      addBtn()!.click()
      flushSync()
      expect(picker()).not.toBeNull()
      expect(offered()).toEqual(['sam'])
    })

    it('offers people sharing a server with you', () => {
      setup({ members: { S1: [{ user: alex, nickname: null }, { user: jo, nickname: null }] } })
      addBtn()!.click()
      flushSync()
      expect(offered()).toEqual(['jo'])
    })

    it('picking people hides you from each of them, then the picker closes', async () => {
      setup({ dmMembers: { D1: ['U1', 'U2'] }, members: { S1: [{ user: alex, nickname: null }, { user: jo, nickname: null }] } })
      addBtn()!.click()
      flushSync()
      for (const n of ['jo', 'sam']) {
        ;[...document.querySelectorAll<HTMLButtonElement>('[role=dialog] .pick')].find((b) => b.textContent?.includes(n))!.click()
        flushSync()
      }
      ;[...document.querySelectorAll<HTMLButtonElement>('[role=dialog] .foot button')].find((b) => b.textContent?.trim() === 'Appear offline')!.click()
      flushSync()
      expect(picker()).toBeNull()
      await vi.waitFor(() => expect(invoke.mock.calls.filter(([c]) => c === 'set_appear_offline').map(([, a]) => a))
        .toEqual([{ userId: 'U4', hidden: true }, { userId: 'U2', hidden: true }]))
    })

    it('no people count in the picker: there is no limit', () => {
      setup({ dmMembers: { D1: ['U1', 'U2'] } })
      addBtn()!.click()
      flushSync()
      expect(document.querySelector('[role=dialog] .foot')!.textContent).not.toMatch(/of \d+ people/)
    })
  })

  it('is a section of Settings', () => {
    cmp = mount(Settings, { target: document.body, props: { onClose: () => {}, section: 'privacy' } })
    flushSync()
    expect([...document.querySelectorAll('nav button')].map((b) => b.textContent)).toContain('Privacy')
    expect(document.querySelector('[aria-current=page]')!.textContent).toBe('Privacy')
    expect(document.querySelector('.content h2')!.textContent).toBe('Privacy')
  })
})
