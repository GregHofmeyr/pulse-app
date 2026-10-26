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
    invoke.mockResolvedValueOnce(3)
    show()
    click('Sign out all other devices')
    click('Sign out')
    await settle(); flushSync()
    expect(calls('logout_others')).toHaveLength(1)
    expect(document.body.textContent).toContain('Signed out 3 other devices')
  })

  it('says one device, not one devices', async () => {
    invoke.mockResolvedValueOnce(1)
    show()
    click('Sign out all other devices')
    click('Sign out')
    await settle(); flushSync()
    expect(document.body.textContent).toContain('Signed out 1 other device')
    expect(document.body.textContent).not.toContain('1 other devices')
  })

  it('says so when there was nothing else signed in', async () => {
    invoke.mockResolvedValueOnce(0)
    show()
    click('Sign out all other devices')
    click('Sign out')
    await settle(); flushSync()
    expect(document.body.textContent).toContain('No other devices were signed in')
  })

  it('shows the error when it fails', async () => {
    invoke.mockRejectedValueOnce("couldn't connect")
    show()
    click('Sign out all other devices')
    click('Sign out')
    await settle(); flushSync()
    expect(document.querySelector('[role="alert"]')?.textContent).toContain("couldn't connect")
  })
})
