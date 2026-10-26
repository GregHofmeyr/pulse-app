// @vitest-environment jsdom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))

import AdvancedSettings from './AdvancedSettings.svelte'

let cmp: ReturnType<typeof mount> | undefined
const show = () => {
  cmp = mount(AdvancedSettings, { target: document.body })
  flushSync()
}
const settle = () => new Promise((r) => setTimeout(r, 0))
const calls = (cmd: string) => invoke.mock.calls.filter((c) => c[0] === cmd)
const button = (t: string) => [...document.querySelectorAll('button')].find((b) => b.textContent === t)
const click = (t: string) => { button(t)!.click(); flushSync() }
/** `logout_others` answers with `r` (other commands as usual). */
const logoutOthers = (r: () => Promise<unknown>) =>
  invoke.mockImplementation(async (cmd: string) =>
    cmd === 'logout_others' ? r() : cmd === 'get_report_settings' ? { auto_send_crashes: true } : undefined)

beforeEach(() => { invoke.mockReset(); invoke.mockResolvedValue(undefined) })
afterEach(() => { if (cmp) unmount(cmp); cmp = undefined; document.body.innerHTML = '' })

describe('AdvancedSettings: sign out other devices', () => {
  it('asks before signing anything out', () => {
    show()
    expect(document.body.textContent).toContain('Sessions')
    click('Sign out all other devices')
    expect(calls('logout_others')).toHaveLength(0)
    expect(document.body.textContent).toMatch(/sure/i)
  })

  it('cancel leaves every device signed in', () => {
    show()
    click('Sign out all other devices')
    click('Cancel')
    expect(calls('logout_others')).toHaveLength(0)
    expect(button('Sign out all other devices')).toBeTruthy()
  })

  it('confirm calls the command and says how many went', async () => {
    logoutOthers(async () => 3)
    show()
    click('Sign out all other devices')
    click('Sign out')
    await settle(); flushSync()
    expect(calls('logout_others')).toHaveLength(1)
    expect(document.body.textContent).toContain('Signed out 3 other devices')
  })

  it('says one device, not one devices', async () => {
    logoutOthers(async () => 1)
    show()
    click('Sign out all other devices')
    click('Sign out')
    await settle(); flushSync()
    expect(document.body.textContent).toContain('Signed out 1 other device')
    expect(document.body.textContent).not.toContain('1 other devices')
  })

  it('says so when there was nothing else signed in', async () => {
    logoutOthers(async () => 0)
    show()
    click('Sign out all other devices')
    click('Sign out')
    await settle(); flushSync()
    expect(document.body.textContent).toContain('No other devices were signed in')
  })

  it('shows the error when it fails', async () => {
    logoutOthers(async () => { throw "couldn't connect" })
    show()
    click('Sign out all other devices')
    click('Sign out')
    await settle(); flushSync()
    expect(document.querySelector('[role="alert"]')?.textContent).toContain("couldn't connect")
  })
})

describe('AdvancedSettings: send crash reports automatically', () => {
  const box = () => [...document.querySelectorAll('label')].find((l) => l.textContent?.includes('Send crash reports automatically'))!
    .querySelector('input') as HTMLInputElement
  const answer = (auto: boolean) =>
    invoke.mockImplementation(async (cmd: string, args?: unknown) =>
      cmd === 'get_report_settings' ? { auto_send_crashes: auto }
      : cmd === 'set_report_settings' ? (args as { settings: unknown }).settings
      : undefined)

  it('shows the saved setting (on by default)', async () => {
    answer(true)
    show()
    await settle(); flushSync()
    expect(box().checked).toBe(true)
    expect(document.body.textContent).toMatch(/asked/i) // the hint says what off means
  })

  it('shows off when it was turned off', async () => {
    answer(false)
    show()
    await settle(); flushSync()
    expect(box().checked).toBe(false)
  })

  it('turning it off saves that', async () => {
    answer(true)
    show()
    await settle(); flushSync()
    box().click()
    await settle(); flushSync()
    expect(calls('set_report_settings')).toEqual([['set_report_settings', { settings: { auto_send_crashes: false } }]])
    expect(box().checked).toBe(false)
  })

  it('a failed save puts it back and says why', async () => {
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'get_report_settings') return { auto_send_crashes: true }
      if (cmd === 'set_report_settings') throw "couldn't save the setting: disk full"
      return undefined
    })
    show()
    await settle(); flushSync()
    box().click()
    await settle(); flushSync()
    expect(box().checked).toBe(true)
    expect(document.querySelector('[role="alert"]')?.textContent).toContain('disk full')
  })

  it("can't be changed before the saved value is known", () => {
    invoke.mockImplementation(() => new Promise(() => {}))
    show()
    expect(box().disabled).toBe(true)
  })
})
