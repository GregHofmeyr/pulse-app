// @vitest-environment jsdom
// Settings → Notifications → Go-live notifications (#60): everyone whose going live you're notified of, each with Remove.
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { fakeMatchMedia, FakeIntersectionObserver } from '../lib/testMotion'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))
vi.stubGlobal('IntersectionObserver', FakeIntersectionObserver)
vi.stubGlobal('matchMedia', fakeMatchMedia(false).matchMedia)

import NotificationSettings from './NotificationSettings.svelte'
import { app, resetState } from '../lib/store.svelte'
import { applyEvent, emptyState } from '../lib/state'

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
    liveNotify: ['U3', 'U2'],
  }
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})
const rows = () => [...document.querySelectorAll<HTMLElement>('.follow-list li')]

describe('go-live notifications in Settings', () => {
  it('lists everyone you follow, by the name you see them by, with their avatar', () => {
    cmp = mount(NotificationSettings, { target: document.body })
    flushSync()
    expect([...document.querySelectorAll('h3')].map((h) => h.textContent)).toContain('Go-live notifications')
    expect(rows().map((r) => r.querySelector('.who')!.textContent?.trim())).toEqual(['Kimmy', 'sam'])
    expect(rows().every((r) => r.querySelector('.av'))).toBe(true)
  })

  it('Remove stops the notifications; the list follows the event', () => {
    cmp = mount(NotificationSettings, { target: document.body })
    flushSync()
    const remove = rows()[1].querySelector<HTMLButtonElement>('button')!
    expect(remove.textContent?.trim()).toBe('Remove')
    expect(remove.getAttribute('aria-label')).toBe('Stop notifying me when sam goes live')
    remove.click()
    flushSync()
    expect(invoke.mock.calls.filter(([c]) => c === 'set_live_notify').map(([, a]) => a)).toEqual([{ userId: 'U2', on: false }])
    app.state = applyEvent(app.state, { t: 'LiveNotifyChanged', d: { user_id: 'U1', target_id: 'U2', on: false } }, 0)
    flushSync()
    expect(rows().map((r) => r.querySelector('.who')!.textContent?.trim())).toEqual(['Kimmy'])
  })

  it('empty: says how to add someone', () => {
    app.state = { ...app.state, liveNotify: [] }
    cmp = mount(NotificationSettings, { target: document.body })
    flushSync()
    expect(rows()).toEqual([])
    expect(document.querySelector('.empty')!.textContent).toContain('Notify me when they go live')
  })

  it('a refused Remove shows why', async () => {
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'set_live_notify') throw { kind: 'http', status: 429, message: 'Too fast' }
      return undefined
    })
    cmp = mount(NotificationSettings, { target: document.body })
    flushSync()
    rows()[0].querySelector<HTMLButtonElement>('button')!.click()
    await new Promise((r) => setTimeout(r, 0))
    flushSync()
    expect(document.querySelector('[role=alert]')).not.toBeNull()
  })
})
