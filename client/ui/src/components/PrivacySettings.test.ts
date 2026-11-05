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

describe('PrivacySettings', () => {
  it('lists everyone you appear offline to, by the name you see them by, with their avatar', () => {
    cmp = mount(PrivacySettings, { target: document.body })
    flushSync()
    expect(document.querySelector('h3')!.textContent).toBe('Appear offline to')
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
    expect(document.querySelector('.empty')!.textContent).toContain("You don't appear offline to anyone")
  })

  it('is a section of Settings', () => {
    cmp = mount(Settings, { target: document.body, props: { onClose: () => {}, section: 'privacy' } })
    flushSync()
    expect([...document.querySelectorAll('nav button')].map((b) => b.textContent)).toContain('Privacy')
    expect(document.querySelector('[aria-current=page]')!.textContent).toBe('Privacy')
    expect(document.querySelector('.content h2')!.textContent).toBe('Privacy')
  })
})
