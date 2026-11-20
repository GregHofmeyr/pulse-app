// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))

import { IDLE_INSTALL_MS, IDLE_POLL_MS, idleInstallDue, startIdleInstall, updates } from './updates.svelte'
import { windowFocus } from './windowFocus.svelte'

const installs = () => invoke.mock.calls.filter((c) => c[0] === 'update_install_idle').length

describe('idle install (#460)', () => {
  it('is due only with an offer, nothing installing or going on, and a long enough quiet spell', () => {
    const v = { offer: true, installing: false, quietMs: IDLE_INSTALL_MS, blocked: false }
    expect(IDLE_INSTALL_MS).toBe(10 * 60_000)
    expect(idleInstallDue(v)).toBe(true)
    expect(idleInstallDue({ ...v, offer: false })).toBe(false)
    expect(idleInstallDue({ ...v, installing: true })).toBe(false)
    expect(idleInstallDue({ ...v, blocked: true })).toBe(false)
    expect(idleInstallDue({ ...v, quietMs: IDLE_INSTALL_MS - 1 })).toBe(false)
  })

  let stop: (() => void) | undefined
  let blocked = false
  beforeEach(() => {
    vi.useFakeTimers()
    invoke.mockReset()
    invoke.mockImplementation(async () => undefined)
    blocked = false
    windowFocus.focused = false
    Object.assign(updates, { offer: { version: '9.9.9' }, installing: false, progress: null, error: null })
    stop = startIdleInstall(() => blocked)
  })
  afterEach(() => {
    stop?.()
    vi.useRealTimers()
  })

  it('installs after ten quiet minutes with an update waiting', async () => {
    await vi.advanceTimersByTimeAsync(IDLE_INSTALL_MS - IDLE_POLL_MS)
    expect(installs()).toBe(0)
    await vi.advanceTimersByTimeAsync(IDLE_POLL_MS)
    expect(installs()).toBe(1)
    expect(updates.installing).toBe(true) // "Updating Pulse…" shows; Pulse closes for the installer
  })

  it('input in the window starts the quiet spell again', async () => {
    await vi.advanceTimersByTimeAsync(IDLE_INSTALL_MS - IDLE_POLL_MS)
    window.dispatchEvent(new KeyboardEvent('keydown'))
    await vi.advanceTimersByTimeAsync(IDLE_INSTALL_MS - IDLE_POLL_MS)
    expect(installs()).toBe(0)
    await vi.advanceTimersByTimeAsync(IDLE_POLL_MS)
    expect(installs()).toBe(1)
  })

  it('never while something is going on (a call, a share, an upload, a draft)', async () => {
    blocked = true
    await vi.advanceTimersByTimeAsync(IDLE_INSTALL_MS * 3)
    expect(installs()).toBe(0)
    blocked = false
    await vi.advanceTimersByTimeAsync(IDLE_POLL_MS)
    expect(installs()).toBe(1)
  })

  it('a refusal carries on quietly and waits another full quiet spell', async () => {
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'update_install_idle') throw new Error('Finish your call first')
      return undefined
    })
    await vi.advanceTimersByTimeAsync(IDLE_INSTALL_MS)
    expect(installs()).toBe(1)
    expect(updates.installing).toBe(false)
    expect(updates.error).toBeNull()
    await vi.advanceTimersByTimeAsync(IDLE_INSTALL_MS - IDLE_POLL_MS)
    expect(installs()).toBe(1)
    await vi.advanceTimersByTimeAsync(IDLE_POLL_MS)
    expect(installs()).toBe(2)
  })

  it('nothing to install, nothing happens', async () => {
    updates.offer = null
    await vi.advanceTimersByTimeAsync(IDLE_INSTALL_MS * 2)
    expect(installs()).toBe(0)
  })
})
