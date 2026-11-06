// @vitest-environment jsdom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const invoke = vi.fn()
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...a: unknown[]) => invoke(...a) }))

import UpdateRequired, { FORCED_RECHECK_MS } from './UpdateRequired.svelte'
import { updates } from '../lib/updates.svelte'

let cmp: ReturnType<typeof mount> | undefined
const text = () => document.body.textContent ?? ''
const retry = () => [...document.querySelectorAll('button')].find((b) => b.textContent?.trim() === 'Retry')
const checks = () => invoke.mock.calls.filter(([c]) => c === 'update_check').length
/** answers: update_check → `check()`, update_install → `install()` */
function core(check: () => Promise<unknown>, install: () => Promise<unknown> = () => new Promise(() => {})) {
  invoke.mockImplementation(async (cmd: string) => (cmd === 'update_check' ? check() : cmd === 'update_install' ? install() : undefined))
}
async function show() {
  cmp = mount(UpdateRequired, { target: document.body })
  flushSync()
  await vi.waitFor(() => expect(checks()).toBeGreaterThan(0))
}

beforeEach(() => invoke.mockReset())
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  updates.offer = null
  updates.installing = false
  updates.error = null
  document.body.innerHTML = ''
  vi.useRealTimers()
})

describe('UpdateRequired (#82)', () => {
  it('forced_installs_automatically: a published update installs without asking', async () => {
    core(async () => ({ version: '0.2.1' }))
    await show()
    await vi.waitFor(() => expect(invoke).toHaveBeenCalledWith('update_install', { restart: false }))
    expect(updates.installing).toBe(true) // App shows "Updating Pulse…" over this screen
    expect(updates.offer).toEqual({ version: '0.2.1' })
  })

  it('forced_without_update_retries: not published yet → says so, re-checks every minute and on Retry', async () => {
    vi.useFakeTimers()
    core(async () => null)
    await show()
    await vi.waitFor(() => expect(text()).toContain("isn't ready yet"))
    expect(text()).toContain('plz update :)')
    expect(retry()).toBeDefined()
    expect(checks()).toBe(1)
    await vi.advanceTimersByTimeAsync(FORCED_RECHECK_MS)
    expect(checks()).toBe(2)
    retry()!.click()
    await vi.waitFor(() => expect(checks()).toBe(3))
    expect(invoke).not.toHaveBeenCalledWith('update_install', expect.anything())
  })

  it('forced_failure_offers_retry: a failed install shows why, and Retry tries again', async () => {
    core(
      async () => ({ version: '0.2.1' }),
      async () => {
        throw 'signature mismatch'
      },
    )
    await show()
    await vi.waitFor(() => expect(text()).toContain("Couldn't update: signature mismatch"))
    expect(updates.installing).toBe(false)
    expect(retry()).toBeDefined()
    retry()!.click()
    await vi.waitFor(() => expect(checks()).toBe(2))
  })

  it('a failed check shows why, with Retry', async () => {
    core(async () => {
      throw 'offline'
    })
    await show()
    await vi.waitFor(() => expect(text()).toContain("Couldn't update: offline"))
    expect(retry()).toBeDefined()
  })

  it('stops re-checking once it is gone', async () => {
    vi.useFakeTimers()
    core(async () => null)
    await show()
    await vi.waitFor(() => expect(retry()).toBeDefined())
    unmount(cmp!)
    cmp = undefined
    await vi.advanceTimersByTimeAsync(FORCED_RECHECK_MS * 3)
    expect(checks()).toBe(1)
  })
})
