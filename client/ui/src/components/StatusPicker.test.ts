// @vitest-environment jsdom
// Your status menu (#45): click your avatar at the bottom of the sidebar. Online, Do Not Disturb (with its timers),
// Invisible, each with its dot and the current one ticked; then Your profile.
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))

import StatusPicker from './StatusPicker.svelte'
import { app, clock, resetState, ui } from '../lib/store.svelte'
import { emptyState } from '../lib/state'
import { tomorrowMorning } from '../lib/status'
import type { OwnStatus } from '../lib/protocol/OwnStatus'

const NOW = Date.UTC(2026, 10, 5, 10)
const alex = { id: 'U1', username: 'alex', display_name: 'alex', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
let cmp: ReturnType<typeof mount> | undefined
let opener: HTMLButtonElement
const onClose = vi.fn()
const onProfile = vi.fn()

function show(myStatus: OwnStatus = { status: 'online', dnd_until: null }) {
  app.state = { ...emptyState(), me: alex, myStatus }
  opener = document.createElement('button')
  document.body.append(opener)
  cmp = mount(StatusPicker, { target: document.body, props: { opener, onClose, onProfile } })
  flushSync()
}
const q = <T extends HTMLElement = HTMLElement>(sel: string) => document.querySelector<T>(sel)
const item = (id: string) => q(`[role=menu] [data-item="${id}"]`)!
const ids = () => [...document.querySelectorAll<HTMLElement>('.status-menu > [data-item]')].map((i) => i.dataset.item)
const key = (el: Element, k: string) => {
  el.dispatchEvent(new KeyboardEvent('keydown', { key: k, bubbles: true, cancelable: true }))
  flushSync()
}
const statusCalls = () => invoke.mock.calls.filter(([c]) => c === 'set_status').map(([, a]) => a)

beforeEach(() => {
  vi.useFakeTimers({ toFake: ['Date'] })
  vi.setSystemTime(NOW)
  clock.nowIso = new Date(NOW).toISOString()
  resetState()
  invoke.mockReset()
  invoke.mockResolvedValue(undefined)
  onClose.mockReset()
  onProfile.mockReset()
  ui.notice = null
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
  vi.useRealTimers()
})

describe('StatusPicker', () => {
  it('the three statuses with their dots, then Your profile; the current one ticked; focus on the first', () => {
    show()
    expect(ids()).toEqual(['online', 'dnd', 'invisible', 'profile'])
    expect(item('online').textContent).toContain('Online')
    expect(item('dnd').textContent).toContain('Do Not Disturb')
    expect(item('invisible').textContent).toContain('Invisible')
    expect(item('profile').textContent).toContain('Your profile')
    for (const k of ['online', 'dnd', 'invisible']) expect(item(k).querySelector(`.dot.${k}`)).not.toBeNull()
    expect(item('online').getAttribute('aria-checked')).toBe('true')
    expect(item('dnd').getAttribute('aria-checked')).toBe('false')
    expect(document.activeElement).toBe(item('online'))
  })

  it('ticks your status; a run-out DND timer ticks Online', () => {
    show({ status: 'invisible', dnd_until: null })
    expect(item('invisible').getAttribute('aria-checked')).toBe('true')
    unmount(cmp!)
    document.body.innerHTML = ''
    show({ status: 'dnd', dnd_until: new Date(NOW - 1).toISOString() })
    expect(item('online').getAttribute('aria-checked')).toBe('true')
  })

  it('Online and Invisible set the status (no timer) and close', () => {
    show({ status: 'dnd', dnd_until: null })
    item('online').click()
    flushSync()
    expect(statusCalls()).toEqual([{ status: 'online', dndUntil: null }])
    expect(onClose).toHaveBeenCalled()
    unmount(cmp!)
    document.body.innerHTML = ''
    show()
    item('invisible').click()
    flushSync()
    expect(statusCalls().at(-1)).toEqual({ status: 'invisible', dndUntil: null })
  })

  it('Do Not Disturb opens its timers; each sends its end time', () => {
    show()
    item('dnd').click()
    flushSync()
    const flyout = q('[role=menu][aria-label="Do Not Disturb for"]')!
    const timers = [...flyout.querySelectorAll<HTMLButtonElement>('[role=menuitem]')]
    expect(timers.map((b) => b.textContent?.trim())).toEqual(['For 1 hour', 'For 8 hours', 'Until tomorrow morning', 'Until I turn it off'])
    expect(document.activeElement).toBe(timers[0])
    timers[0].click()
    flushSync()
    expect(statusCalls()).toEqual([{ status: 'dnd', dndUntil: new Date(NOW + 3_600_000).toISOString() }])
    expect(onClose).toHaveBeenCalled()
  })

  it('Until tomorrow morning and Until I turn it off', () => {
    show()
    item('dnd').click()
    flushSync()
    const pick = (label: string) => [...document.querySelectorAll<HTMLButtonElement>('[aria-label="Do Not Disturb for"] [role=menuitem]')].find((b) => b.textContent?.trim() === label)!.click()
    pick('Until tomorrow morning')
    expect(statusCalls().at(-1)).toEqual({ status: 'dnd', dndUntil: tomorrowMorning(new Date(NOW)).toISOString() })
    unmount(cmp!)
    document.body.innerHTML = ''
    show()
    item('dnd').click()
    flushSync()
    pick('Until I turn it off')
    expect(statusCalls().at(-1)).toEqual({ status: 'dnd', dndUntil: null })
  })

  it('the keyboard: arrows move, ArrowRight opens the timers, ArrowLeft goes back, Escape closes', () => {
    show()
    key(document.activeElement!, 'ArrowDown')
    expect(document.activeElement).toBe(item('dnd'))
    key(document.activeElement!, 'ArrowRight')
    expect(document.activeElement?.textContent?.trim()).toBe('For 1 hour')
    key(document.activeElement!, 'ArrowLeft')
    expect(document.activeElement).toBe(item('dnd'))
    expect(q('[aria-label="Do Not Disturb for"]')).toBeNull()
    key(document.activeElement!, 'Escape')
    expect(onClose).toHaveBeenCalled()
    expect(document.activeElement).toBe(opener)
  })

  it('Your profile does what the avatar did', () => {
    show()
    item('profile').click()
    flushSync()
    expect(onProfile).toHaveBeenCalled()
    expect(onClose).toHaveBeenCalled()
    expect(statusCalls()).toEqual([])
  })

  it('a failure says why', async () => {
    invoke.mockRejectedValue({ code: 'network', message: 'no connection' })
    show()
    item('invisible').click()
    await vi.waitFor(() => expect(ui.notice).toBeTruthy())
  })
})
