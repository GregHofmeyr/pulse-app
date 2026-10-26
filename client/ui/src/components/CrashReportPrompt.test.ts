// @vitest-environment jsdom
// "Send crash reports automatically" off: last session's report waits, and the user decides.
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))
const listeners = new Map<string, (e: { payload: unknown }) => void>()
vi.mock('@tauri-apps/api/event', () => ({
  listen: async (name: string, fn: (e: { payload: unknown }) => void) => {
    listeners.set(name, fn)
    return () => {}
  },
}))

import CrashReportPrompt from './CrashReportPrompt.svelte'
import { startListening, ui } from '../lib/store.svelte'

let cmp: ReturnType<typeof mount> | undefined
const show = () => {
  cmp = mount(CrashReportPrompt, { target: document.body })
  flushSync()
}
const settle = async () => { await new Promise((r) => setTimeout(r, 0)); flushSync() }
const calls = (cmd: string) => invoke.mock.calls.filter((c) => c[0] === cmd)
const button = (t: string) => [...document.querySelectorAll('button')].find((b) => b.textContent === t)
const click = (t: string) => { button(t)!.click(); flushSync() }
const text = () => document.body.textContent ?? ''

beforeEach(() => { invoke.mockReset(); invoke.mockResolvedValue(undefined); ui.heldReport = null })
afterEach(() => { if (cmp) unmount(cmp); cmp = undefined; document.body.innerHTML = '' })

describe('held crash report prompt', () => {
  it('shows nothing when no report is waiting', () => {
    show()
    expect(button('Send')).toBeUndefined()
    expect(text()).toBe('')
  })

  it('appears when the core says a report was held', async () => {
    await startListening()
    show()
    listeners.get('pulse://crash-report-held')!({ payload: { kind: 'crash' } })
    flushSync()
    expect(text()).toMatch(/Pulse crashed last time/)
    expect(button('Send')).toBeTruthy()
    expect(button("Don't send")).toBeTruthy()
    expect(calls('send_held_report')).toHaveLength(0) // asking only: nothing sent yet
  })

  it('names an unexpected exit as such', () => {
    ui.heldReport = 'unexpected-exit'
    show()
    expect(text()).toMatch(/closed unexpectedly last time/)
  })

  it('Send sends it, then the prompt goes and says so', async () => {
    invoke.mockResolvedValueOnce('TVWXYZ')
    ui.heldReport = 'crash'
    show()
    click('Send')
    await settle()
    expect(calls('send_held_report')).toHaveLength(1)
    expect(calls('discard_held_report')).toHaveLength(0)
    expect(ui.heldReport).toBeNull()
    expect(button('Send')).toBeUndefined()
    expect(text()).toMatch(/sent/i)
  })

  it("a failed Send keeps the prompt and shows why, so they can retry or not send", async () => {
    invoke.mockRejectedValueOnce("Couldn't send the report (can't reach the server). It's kept on this computer")
    ui.heldReport = 'crash'
    show()
    click('Send')
    await settle()
    expect(ui.heldReport).toBe('crash')
    expect(document.querySelector('[role="alert"]')?.textContent).toMatch(/kept on this computer/)
    expect(button('Send')).toBeTruthy()
    expect(button("Don't send")).toBeTruthy()
  })

  it("Don't send deletes it without sending", async () => {
    ui.heldReport = 'crash'
    show()
    click("Don't send")
    await settle()
    expect(calls('discard_held_report')).toHaveLength(1)
    expect(calls('send_held_report')).toHaveLength(0)
    expect(ui.heldReport).toBeNull()
    expect(button('Send')).toBeUndefined()
  })
})
