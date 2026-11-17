// @vitest-environment jsdom
// Per-person Do Not Disturb (#310): the rule (no sound or notification from them, mentions included; only in DMs when
// asked; a timed one ends at its time), the person menu's item (a flyout of how long; on, a line saying so and Turn
// off), and Settings → Notifications' list with + Add person, where it applies, and Remove.
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { fakeMatchMedia, FakeIntersectionObserver } from '../lib/testMotion'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))
vi.stubGlobal('IntersectionObserver', FakeIntersectionObserver)
vi.stubGlobal('matchMedia', fakeMatchMedia(false).matchMedia)

import MemberMenu from './MemberMenu.svelte'
import NotificationSettings from './NotificationSettings.svelte'
import { app, clock, resetState } from '../lib/store.svelte'
import { applyEvent, emptyState } from '../lib/state'
import { shouldAlert, type AlertInput } from '../lib/conversations'
import { personDndOf, quietFrom } from '../lib/personDnd'
import type { Message } from '../lib/protocol/Message'
import type { Channel } from '../lib/protocol/Channel'
import type { PersonDnd } from '../lib/protocol/PersonDnd'

const user = (id: string, name: string) => ({ id, username: name, display_name: name, avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } })
const alex = user('U1', 'alex')
const sam = user('U2', 'sam')
const kim = user('U3', 'kim')
const NOW = '2026-11-17T12:00:00.000Z'
const later = '2026-11-17T13:00:00.000Z'
const earlier = '2026-11-17T11:00:00.000Z'

describe('the rule', () => {
  const list: PersonDnd[] = [
    { user_id: 'U2', dms_only: false, until: null },
    { user_id: 'U3', dms_only: true, until: later },
    { user_id: 'U4', dms_only: false, until: earlier },
  ]
  it('in force until its time; only in DMs when asked', () => {
    expect(personDndOf(list, 'U2', NOW)).not.toBeNull()
    expect(personDndOf(list, 'U4', NOW)).toBeNull()
    expect(quietFrom(list, 'U2', false, NOW)).toBe(true)
    expect(quietFrom(list, 'U3', true, NOW)).toBe(true)
    expect(quietFrom(list, 'U3', false, NOW)).toBe(false)
    expect(quietFrom(list, 'U3', true, later)).toBe(false)
    expect(quietFrom(list, 'U4', true, NOW)).toBe(false)
    expect(quietFrom(list, null, true, NOW)).toBe(false)
  })

  it('a quiet author alerts nothing, a mention included; anyone else as before', () => {
    const m = { id: 'M1', channel_id: 'D1', author_id: 'U2', kind: 'normal', content: '@alex hi', mentions: ['U1'], deleted: false, thread_root_id: null } as unknown as Message
    const dm = { id: 'D1', server_id: null, kind: 'dm', name: null, position: 0 } as unknown as Channel
    const v: AlertInput = { message: m, me: 'U1', channel: dm, open: false, openThread: null, focused: false, muted: false, level: 'all' }
    expect(shouldAlert(v)).toBe(true)
    expect(shouldAlert({ ...v, quiet: true })).toBe(false)
  })
})

let cmp: ReturnType<typeof mount> | undefined
beforeEach(() => {
  resetState()
  invoke.mockReset()
  invoke.mockResolvedValue(undefined)
  vi.useFakeTimers({ now: new Date(NOW), toFake: ['Date'] })
  clock.nowIso = NOW
  app.state = {
    ...emptyState(), me: alex,
    people: {
      U1: { user: alex, online: true, last_seen_at: null, status: 'online' },
      U2: { user: sam, online: true, last_seen_at: null, status: 'online' },
      U3: { user: kim, online: false, last_seen_at: null, status: 'offline' },
    },
    members: { S1: [{ user: alex, nickname: null }, { user: sam, nickname: null }, { user: kim, nickname: null }] },
  } as never
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
  vi.useRealTimers()
})

describe('the person menu', () => {
  const open = (userId = 'U2') => {
    const opener = document.createElement('button')
    document.body.append(opener)
    cmp = mount(MemberMenu, { target: document.body, props: { serverId: 'S1', userId, x: 10, y: 10, opener, onClose: () => {} } })
    flushSync()
  }
  const item = () => document.querySelector<HTMLElement>('[role=menu] [data-item="person-dnd"]')

  it('off: a flyout of how long; picking one turns it on (and only in DMs from its last choice)', async () => {
    invoke.mockImplementation(async (cmd: string) => (cmd === 'set_person_dnd' ? [{ user_id: 'U2', dms_only: false, until: later }] : undefined))
    open()
    expect(item()!.textContent).toContain('Do Not Disturb for them')
    item()!.click()
    flushSync()
    const flyout = document.querySelector<HTMLElement>('[role=menu][aria-label="Do Not Disturb for this person"]')!
    const choices = [...flyout.querySelectorAll<HTMLButtonElement>('[role=menuitem]')]
    expect(choices.map((c) => c.textContent?.trim()).at(-1)).toBe('Only in DMs, until I turn it back on')
    choices.find((c) => c.textContent?.trim() === 'For 1 hour')!.click()
    await vi.waitFor(() => expect(app.state.personDnd).toHaveLength(1))
    expect(invoke).toHaveBeenCalledWith('set_person_dnd', { userId: 'U2', request: { dms_only: false, until: later } })
  })

  it('on: a line says so and how long, and Turn off turns it off', async () => {
    app.state = { ...app.state, personDnd: [{ user_id: 'U2', dms_only: true, until: null }] }
    invoke.mockImplementation(async (cmd: string) => (cmd === 'set_person_dnd' ? [] : undefined))
    open()
    expect(document.querySelector('[data-person-dnd]')!.textContent).toContain('Do Not Disturb in DMs · until you turn it off')
    expect(item()!.textContent).toBe('Turn off Do Not Disturb for them')
    item()!.click()
    await vi.waitFor(() => expect(app.state.personDnd).toEqual([]))
    expect(invoke).toHaveBeenCalledWith('set_person_dnd', { userId: 'U2', request: null })
  })

  it('never for yourself; a run-out one reads as off', () => {
    app.state = { ...app.state, personDnd: [{ user_id: 'U2', dms_only: false, until: earlier }] }
    open('U1')
    expect(item()).toBeNull()
    unmount(cmp!)
    document.body.innerHTML = ''
    open('U2')
    expect(item()!.textContent).toContain('Do Not Disturb for them')
    expect(document.querySelector('[data-person-dnd]')).toBeNull()
  })
})

describe('Settings → Notifications', () => {
  const rows = () => [...document.querySelectorAll<HTMLElement>('.quiet-list li')]
  const show = () => {
    cmp = mount(NotificationSettings, { target: document.body })
    flushSync()
  }

  it('lists them by name with how long; Remove and where it applies change it; the list follows the event', async () => {
    app.state = { ...app.state, personDnd: [{ user_id: 'U3', dms_only: false, until: null }, { user_id: 'U2', dms_only: true, until: later }] }
    invoke.mockImplementation(async (cmd: string) => (cmd === 'set_person_dnd' ? app.state.personDnd : undefined))
    show()
    expect(rows().map((r) => r.querySelector('.who')!.textContent)).toEqual(['kim', 'sam'])
    expect(rows()[0].querySelector('.until')!.textContent).toBe('Until you turn it off')
    const where = rows()[1].querySelector<HTMLSelectElement>('select')!
    expect(where.value).toBe('dms')
    where.value = 'all'
    where.dispatchEvent(new Event('change', { bubbles: true }))
    expect(invoke).toHaveBeenCalledWith('set_person_dnd', { userId: 'U2', request: { dms_only: false, until: later } })
    rows()[0].querySelector<HTMLButtonElement>('button')!.click()
    expect(invoke).toHaveBeenCalledWith('set_person_dnd', { userId: 'U3', request: null })
    app.state = applyEvent(app.state, { t: 'PersonDndChanged', d: { user_id: 'U1', list: [{ user_id: 'U2', dms_only: false, until: later }] } }, 0)
    flushSync()
    expect(rows().map((r) => r.querySelector('.who')!.textContent)).toEqual(['sam'])
  })

  it('+ Add person opens the people picker with those not on it; picking turns it on everywhere', async () => {
    app.state = { ...app.state, personDnd: [{ user_id: 'U3', dms_only: false, until: null }] }
    show()
    expect(rows()).toHaveLength(1)
    ;[...document.querySelectorAll<HTMLButtonElement>('button')].find((b) => b.textContent?.includes('Add person'))!.click()
    flushSync()
    const dialog = document.querySelector<HTMLElement>('.overlay')!
    expect(dialog.textContent).toContain('sam')
    expect(dialog.textContent).not.toContain('kim')
  })

  it('empty: says how to add someone', () => {
    show()
    expect(rows()).toEqual([])
    expect([...document.querySelectorAll('.empty')].map((e) => e.textContent).join(' ')).toContain('Do Not Disturb for them')
  })
})
