// @vitest-environment jsdom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const invoke = vi.fn()
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...a: unknown[]) => invoke(...a) }))

import UpdateSettings from './UpdateSettings.svelte'
import { updates } from '../lib/updates.svelte'
import { voice } from '../lib/voice.svelte'

let cmp: ReturnType<typeof mount> | undefined
const text = () => document.body.textContent ?? ''
const button = (t: string) => [...document.querySelectorAll('button')].find((b) => b.textContent?.trim() === t)
const WINDOWS = { version: '0.2.14', enabled: true, packageManager: false }
/** answers: update_info → `info`, update_check → `check()` */
function core(info: unknown, check: () => Promise<unknown> = async () => null) {
  invoke.mockImplementation(async (cmd: string) =>
    cmd === 'update_info' ? info : cmd === 'update_check' ? check() : cmd === 'update_install' ? new Promise(() => {}) : undefined)
}
async function show() {
  cmp = mount(UpdateSettings, { target: document.body })
  flushSync()
  await vi.waitFor(() => expect(invoke).toHaveBeenCalledWith('update_info'))
  await new Promise((r) => setTimeout(r, 0))
  flushSync()
}
async function checkNow() {
  button('Check for updates')!.click()
  await vi.waitFor(() => expect(invoke).toHaveBeenCalledWith('update_check'))
  await new Promise((r) => setTimeout(r, 0))
  flushSync()
}

beforeEach(() => invoke.mockReset())
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  updates.offer = null
  updates.installing = false
  updates.error = null
  voice.channelId = null
  document.body.innerHTML = ''
})

describe('UpdateSettings (#460)', () => {
  it('a check that finds nothing says so', async () => {
    core(WINDOWS)
    await show()
    expect(text()).toContain('Pulse 0.2.14')
    await checkNow()
    expect(document.querySelector('[role=status]')?.textContent).toBe("You're up to date.")
  })

  it('a check that finds a version offers Restart to update, which installs', async () => {
    core(WINDOWS, async () => ({ version: '0.2.15' }))
    await show()
    await checkNow()
    expect(text()).toContain('Pulse 0.2.15 is ready.')
    expect(updates.offer).toEqual({ version: '0.2.15' }) // the title bar's button too
    button('Restart to update')!.click()
    expect(invoke).toHaveBeenCalledWith('update_install', { restart: true })
  })

  it('Restart to update waits while you are in a call', async () => {
    core(WINDOWS, async () => ({ version: '0.2.15' }))
    voice.channelId = 'c1'
    await show()
    await checkNow()
    const b = button('Restart to update')!
    expect(b.getAttribute('aria-disabled')).toBe('true')
    expect(text()).toContain('Finish your call first.')
    b.click()
    expect(invoke).not.toHaveBeenCalledWith('update_install', expect.anything())
  })

  it('a failed check says why', async () => {
    core(WINDOWS, async () => { throw 'the update check timed out' })
    await show()
    await checkNow()
    expect(document.querySelector('[role=alert]')?.textContent).toBe("Couldn't check for updates: the update check timed out")
    expect(button('Check for updates')).toBeTruthy()
  })

  it('on Linux updates come from the package manager, with no button', async () => {
    core({ version: '0.2.14', enabled: false, packageManager: true })
    await show()
    expect(text()).toContain('Updates come from your package manager.')
    expect(button('Check for updates')).toBeUndefined()
  })

  it('a Windows build that does not update itself shows only its version', async () => {
    core({ version: '0.2.14', enabled: false, packageManager: false })
    await show()
    expect(text()).toContain('Pulse 0.2.14')
    expect(text()).not.toContain('package manager')
    expect(button('Check for updates')).toBeUndefined()
  })
})
