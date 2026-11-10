// @vitest-environment jsdom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const h = vi.hoisted(() => ({
  restore: { status: 'refused' } as unknown,
  handlers: {} as Record<string, (e: { payload: unknown }) => void>,
  calls: [] as string[],
  args: {} as Record<string, unknown>,
  /** Commands answered by the test (#82: update_launch, update_install). */
  respond: {} as Record<string, (args: unknown) => Promise<unknown>>,
}))
vi.mock('@tauri-apps/api/core', () => ({
  invoke: async (cmd: string, args?: unknown) => {
    h.calls.push(cmd)
    h.args[cmd] = args
    if (h.respond[cmd]) return h.respond[cmd](args)
    return cmd === 'restore_session' ? h.restore : undefined
  },
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
import { app, ui } from './lib/store.svelte'
import { emptyState } from './lib/state'
import { updates } from './lib/updates.svelte'

const alex = { id: 'U1', username: 'alex', display_name: 'alex', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
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
  h.calls = []
  h.args = {}
  h.respond = {}
  // an install that "succeeded" (Pulse would close) leaves the Updating screen up: start each test without it
  Object.assign(updates, { offer: null, installing: false, progress: null, error: null })
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

  it('Sign out on the waiting screen logs out and goes to login with a clean state', async () => {
    await start({ status: 'refused' })
    conn('too_many_devices')
    app.state = { ...app.state, me: alex }
    const out = [...document.querySelectorAll('button')].find((b) => b.textContent === 'Sign out')
    expect(out).toBeTruthy()
    out!.click()
    await vi.waitFor(() => expect(document.querySelector('form')).not.toBeNull())
    expect(h.calls).toContain('logout')
    expect(document.querySelector('.waiting')).toBeNull()
    expect(app.state.me).toBeNull()
    expect(text()).not.toContain('You were signed out') // they chose to: no notice
    // the core resets to connecting / refusal states after a logout: still login
    conn('connecting')
    conn('too_many_devices')
    expect(document.querySelector('.waiting')).toBeNull()
  })

  it('a plain logged-out start shows login even while a refusal state is showing', async () => {
    await start({ status: 'logged_out' })
    conn('too_many_devices')
    expect(document.querySelector('.waiting')).toBeNull()
    expect(document.querySelector('form')).not.toBeNull()
  })
})

describe('App: a held crash report', () => {
  const held = (kind: string) => {
    h.handlers['pulse://crash-report-held']({ payload: { kind } })
    flushSync()
  }
  const prompt = () => [...document.querySelectorAll('button')].find((b) => b.textContent === "Don't send")

  it('a restored session shows the Send / Don\'t send prompt in the main window', async () => {
    await start({ status: 'ready', user: alex })
    await vi.waitFor(() => expect(document.querySelector('nav')).not.toBeNull())
    held('crash')
    expect(text()).toContain('Pulse crashed last time')
    expect(prompt()).toBeTruthy()
  })

  it('the event arriving before the main window still shows once it opens', async () => {
    h.restore = { status: 'ready', user: alex }
    app.state = emptyState()
    cmp = mount(App, { target: document.body })
    await vi.waitFor(() => expect(h.handlers['pulse://crash-report-held']).toBeDefined())
    held('unexpected-exit') // emitted as soon as the UI listens, maybe before restore resolves
    await vi.waitFor(() => expect(document.querySelector('nav')).not.toBeNull())
    flushSync()
    expect(text()).toContain('Pulse closed unexpectedly last time')
  })

  it('signing out forgets the prompt: the next sign-in (maybe another server) asks afresh', async () => {
    await start({ status: 'ready', user: alex })
    await vi.waitFor(() => expect(document.querySelector('nav')).not.toBeNull())
    held('crash')
    conn('logged_out')
    expect(document.querySelector('form')).not.toBeNull()
    expect(ui.heldReport).toBeNull()
  })
})

describe('animation freeze', () => {
  it('body_is_unfocused_while_the_window_is_blurred_even_on_the_login_screen', async () => {
    await start({ status: 'logged_out' })
    expect(document.querySelector('form')).not.toBeNull() // past boot: the login screen
    window.dispatchEvent(new Event('focus'))
    flushSync()
    expect(document.body.classList.contains('unfocused')).toBe(false)
    window.dispatchEvent(new Event('blur'))
    flushSync()
    expect(document.body.classList.contains('unfocused')).toBe(true)
    window.dispatchEvent(new Event('focus'))
    flushSync()
    expect(document.body.classList.contains('unfocused')).toBe(false)
  })
})

describe('App: an update at launch (#82)', () => {
  const updating = () => document.querySelector('.updating')

  it('an update at launch shows Updating and installs', async () => {
    h.respond.update_launch = async () => ({ version: '0.2.1' })
    h.respond.update_install = () => new Promise(() => {}) // Pulse closes and reopens: install never comes back
    h.restore = { status: 'ready', user: alex }
    app.state = emptyState()
    cmp = mount(App, { target: document.body })
    await vi.waitFor(() => expect(updating()).not.toBeNull())
    expect(text()).toContain('Updating Pulse…')
    expect(h.calls).toContain('update_install')
    expect(h.args.update_install).toEqual({ restart: false })
    expect(h.calls).not.toContain('restore_session')
  })

  it('a failed launch update boots normally', async () => {
    h.respond.update_launch = async () => ({ version: '0.2.1' })
    h.respond.update_install = async () => {
      throw 'signature mismatch'
    }
    await start({ status: 'ready', user: alex })
    await vi.waitFor(() => expect(document.querySelector('nav')).not.toBeNull())
    expect(h.calls).toContain('update_install')
    expect(h.calls).toContain('restore_session')
    expect(updating()).toBeNull()
  })

  it('unauthorized_check_boots_normally: no offer (a 401, signed out, no update) means no Updating screen', async () => {
    h.respond.update_launch = async () => null
    await start({ status: 'ready', user: alex })
    await vi.waitFor(() => expect(document.querySelector('nav')).not.toBeNull())
    expect(h.calls).toContain('update_launch')
    expect(h.calls).not.toContain('update_install')
    expect(updating()).toBeNull()
  })

  it('a launch check that fails outright boots normally too', async () => {
    h.respond.update_launch = async () => {
      throw 'no core'
    }
    await start({ status: 'logged_out' })
    expect(document.querySelector('form')).not.toBeNull()
    expect(updating()).toBeNull()
  })

  it('asks for the launch update before restoring the session', async () => {
    await start({ status: 'logged_out' })
    expect(h.calls.indexOf('update_launch')).toBeGreaterThanOrEqual(0)
    expect(h.calls.indexOf('update_launch')).toBeLessThan(h.calls.indexOf('restore_session'))
  })
})
