// @vitest-environment jsdom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, describe, expect, it, vi } from 'vitest'

vi.mock('@tauri-apps/api/core', () => ({ invoke: async () => undefined, convertFileSrc: (s: string) => s }))
vi.mock('@tauri-apps/api/event', () => ({ listen: async () => () => {} }))
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({ onFocusChanged: async () => () => {}, isFocused: async () => true, minimize() {}, toggleMaximize() {}, close() {}, startDragging() {} }),
}))

import Shell from './Shell.svelte'
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
})
