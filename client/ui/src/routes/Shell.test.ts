// @vitest-environment jsdom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, describe, expect, it, vi } from 'vitest'

vi.mock('@tauri-apps/api/core', () => ({ invoke: async () => undefined, convertFileSrc: (s: string) => s }))
vi.mock('@tauri-apps/api/event', () => ({ listen: async () => () => {} }))
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({ onFocusChanged: async () => () => {}, isFocused: async () => true, minimize() {}, toggleMaximize() {}, close() {}, startDragging() {} }),
}))

import Shell from './Shell.svelte'
import Waiting from './Waiting.svelte'
import { app } from '../lib/store.svelte'
import { emptyState } from '../lib/state'

let cmp: ReturnType<typeof mount> | undefined
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})

describe('Shell', () => {
  it('server settings and the server menu close when you switch server', () => {
    app.state = { ...emptyState(), servers: [{ id: 'S1', name: 'Alpha', icon_hash: null }, { id: 'S2', name: 'Beta', icon_hash: null }] }
    cmp = mount(Shell, { target: document.body, props: { user: { id: 'U1', username: 'alex', avatar_hash: null }, onLogout: () => {} } })
    flushSync()
    const tab = (name: string) => [...document.querySelectorAll<HTMLButtonElement>('.tab')].find((t) => t.textContent!.includes(name))!
    tab('Alpha').click()
    flushSync()
    document.querySelector<HTMLButtonElement>('[data-menu-toggle]')!.click()
    flushSync()
    ;[...document.querySelectorAll<HTMLButtonElement>('[role=menuitem]')].find((b) => b.textContent === 'Server settings')!.click()
    flushSync()
    expect(document.querySelector('[role=dialog][aria-label="Server settings"]')).not.toBeNull()
    tab('Beta').click()
    flushSync()
    expect(document.querySelector('[role=dialog][aria-label="Server settings"]')).toBeNull()
    expect(document.querySelector('[role=menu]')).toBeNull()
  })

  it('the top bar says why a refused connection is retrying', () => {
    app.state = { ...emptyState(), conn: 'too_many_devices' }
    cmp = mount(Shell, { target: document.body, props: { user: { id: 'U1', username: 'alex', avatar_hash: null }, onLogout: () => {} } })
    flushSync()
    expect(document.querySelector('.conn')!.textContent).toBe('Too many devices connected — close Pulse on another device')
    app.connAt = Date.now()
    app.state = { ...app.state, conn: { rate_limited: 4 } }
    flushSync()
    expect(document.querySelector('.conn')!.textContent).toBe('Too many attempts, retrying in 4s…')
  })

  it('the retry countdown ticks down and is replaced when the next attempt starts', () => {
    vi.useFakeTimers()
    try {
      vi.setSystemTime(100_000)
      app.connAt = Date.now()
      app.state = { ...emptyState(), conn: { rate_limited: 3 } }
      cmp = mount(Shell, { target: document.body, props: { user: { id: 'U1', username: 'alex', avatar_hash: null }, onLogout: () => {} } })
      flushSync()
      expect(document.querySelector('.conn')!.textContent).toBe('Too many attempts, retrying in 3s…')
      vi.advanceTimersByTime(1000)
      flushSync()
      expect(document.querySelector('.conn')!.textContent).toBe('Too many attempts, retrying in 2s…')
      vi.advanceTimersByTime(2000)
      flushSync()
      expect(document.querySelector('.conn')!.textContent).toBe('Connecting…')
    } finally {
      vi.useRealTimers()
    }
  })

  it('a refused startup shows the message on the waiting screen', () => {
    app.connAt = Date.now()
    app.state = { ...emptyState(), conn: 'too_many_devices' }
    cmp = mount(Waiting, { target: document.body, props: { onSignOut: () => {} } })
    flushSync()
    expect(document.querySelector('[role=status]')!.textContent).toContain('Too many devices connected')
  })
})
