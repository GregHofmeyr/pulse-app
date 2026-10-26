// @vitest-environment jsdom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const h = vi.hoisted(() => ({ restore: { status: 'refused' } as unknown, handlers: {} as Record<string, (e: { payload: unknown }) => void> }))
vi.mock('@tauri-apps/api/core', () => ({
  invoke: async (cmd: string) => (cmd === 'restore_session' ? h.restore : undefined),
  convertFileSrc: (s: string) => s,
}))
vi.mock('@tauri-apps/api/event', () => ({
  listen: async (name: string, cb: (e: { payload: unknown }) => void) => {
    h.handlers[name] = cb
    return () => {}
  },
}))
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({ onFocusChanged: async () => () => {}, isFocused: async () => true, minimize() {}, toggleMaximize() {}, close() {}, startDragging() {} }),
}))

import App from './App.svelte'
import { app } from './lib/store.svelte'
import { emptyState } from './lib/state'

const alex = { id: 'U1', username: 'alex', avatar_hash: null }
let cmp: ReturnType<typeof mount> | undefined
const conn = (payload: unknown) => {
  h.handlers['pulse://conn']({ payload })
  flushSync()
}
const text = () => document.body.textContent ?? ''
async function start(restore: unknown) {
  h.restore = restore
  app.state = emptyState()
  cmp = mount(App, { target: document.body })
  await vi.waitFor(() => expect(h.handlers['pulse://conn']).toBeDefined())
  await vi.waitFor(() => expect(document.querySelector('.boot')).toBeNull())
}

beforeEach(() => {
  h.handlers = {}
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})

describe('App: startup refused by the server', () => {
  it('waits with the message, survives the state moving on, and opens on Ready', async () => {
    await start({ status: 'refused' })
    conn('too_many_devices')
    expect(text()).toContain('Too many devices connected')
    conn('connecting') // a short 429 wait ended: still waiting, not Login
    expect(document.querySelector('.waiting')).not.toBeNull()
    expect(document.querySelector('form')).toBeNull()
    // Ready arrives: it carries who we are
    app.state = { ...app.state, me: alex }
    conn('connected')
    await vi.waitFor(() => expect(document.querySelector('.waiting')).toBeNull())
    expect(document.querySelector('nav')).not.toBeNull()
  })

  it('does not depend on the connection state at the moment restore resolves', async () => {
    // conn already moved on to Connecting before the UI saw the (late) restore result
    await start({ status: 'refused' })
    conn('connecting')
    expect(document.querySelector('.waiting')).not.toBeNull()
  })

  it('an update-required answer goes to the update screen', async () => {
    await start({ status: 'refused' })
    conn('too_many_devices')
    conn('update_required')
    expect(document.querySelector('.waiting')).toBeNull()
    expect(text()).toMatch(/update/i)
  })

  it('a dead session shows login with the signed-out notice, and the flag does not linger', async () => {
    await start({ status: 'refused' })
    conn('too_many_devices')
    conn('logged_out')
    expect(document.querySelector('.waiting')).toBeNull()
    expect(text()).toContain('You were signed out')
    // later states (e.g. after a manual logout resets to connecting) stay on login
    conn('connecting')
    conn('too_many_devices')
    expect(document.querySelector('.waiting')).toBeNull()
    expect(document.querySelector('form')).not.toBeNull()
  })

  it('a plain logged-out start shows login even while a refusal state is showing', async () => {
    await start({ status: 'logged_out' })
    conn('too_many_devices')
    expect(document.querySelector('.waiting')).toBeNull()
    expect(document.querySelector('form')).not.toBeNull()
  })
})
