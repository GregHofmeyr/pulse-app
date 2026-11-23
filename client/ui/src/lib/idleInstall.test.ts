// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))

import { IDLE_INSTALL_MS, IDLE_POLL_MS, idleInstallDue, pendingBlocks, startIdleInstall, updates } from './updates.svelte'
import { hasUnsavedEdits, markEdit } from './openEdits'
import type { Pending } from './state'
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

  it('waking from sleep counts as someone being there, not a quiet spell', async () => {
    await vi.advanceTimersByTimeAsync(IDLE_POLL_MS)
    // the computer sleeps for two hours: no timer runs meanwhile, the clock moves on
    vi.setSystemTime(Date.now() + 2 * 3600_000)
    await vi.advanceTimersByTimeAsync(IDLE_POLL_MS)
    expect(installs()).toBe(0)
    await vi.advanceTimersByTimeAsync(IDLE_INSTALL_MS - IDLE_POLL_MS)
    expect(installs()).toBe(0)
    // a full quiet spell after waking does install
    await vi.advanceTimersByTimeAsync(IDLE_POLL_MS)
    expect(installs()).toBe(1)
  })

  it('a late tick within two polls is not taken for a sleep', async () => {
    await vi.advanceTimersByTimeAsync(IDLE_INSTALL_MS - IDLE_POLL_MS)
    vi.setSystemTime(Date.now() + IDLE_POLL_MS / 2) // a busy main thread, not a sleep
    await vi.advanceTimersByTimeAsync(IDLE_POLL_MS)
    expect(installs()).toBe(1)
  })

  it('nothing to install, nothing happens', async () => {
    updates.offer = null
    await vi.advanceTimersByTimeAsync(IDLE_INSTALL_MS * 2)
    expect(installs()).toBe(0)
  })
})

describe('what holds the idle install back (#460)', () => {
  const p = (status: Pending['status'], files = false): Pending => ({
    nonce: 'n', content: 'hi', reply_to_id: null, thread_root_id: null, status, created_at: '',
    ...(files ? { files: [{ id: 'f', name: 'a.png', size: 1, kind: 'image', preview: null }] as Pending['files'] } : {}),
  })
  it('a message still sending, or a failed one with files; never a failed text message (it survives a restart)', () => {
    expect(pendingBlocks({})).toBe(false)
    expect(pendingBlocks({ C1: [] })).toBe(false)
    expect(pendingBlocks({ C1: [p('failed')] })).toBe(false)
    expect(pendingBlocks({ C1: [p('failed'), p('pending')] })).toBe(true)
    expect(pendingBlocks({ C1: [p('failed', true)] })).toBe(true)
  })
  it('an open message edit with changes', () => {
    const a = Symbol('a')
    const b = Symbol('b')
    expect(hasUnsavedEdits()).toBe(false)
    markEdit(a, true)
    markEdit(b, true)
    markEdit(a, false)
    expect(hasUnsavedEdits()).toBe(true)
    markEdit(b, false)
    expect(hasUnsavedEdits()).toBe(false)
  })
})
