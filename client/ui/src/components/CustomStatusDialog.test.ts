// @vitest-environment jsdom
// Your custom status dialog (#152): text and an emoji, when it clears, recent ones to pick again.
import { flushSync, mount, tick, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => undefined) }))

import { invoke } from '@tauri-apps/api/core'
import CustomStatusDialog from './CustomStatusDialog.svelte'
import { app, clock } from '../lib/store.svelte'
import { emptyState } from '../lib/state'
import { endOfToday } from '../lib/customStatus'
import type { MyCustomStatus } from '../lib/protocol/MyCustomStatus'

const NOW = new Date(2026, 10, 18, 15, 20)
const alex = { id: 'U1', username: 'alex', display_name: 'alex', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
const calls = (cmd: string) => vi.mocked(invoke).mock.calls.filter((c) => c[0] === cmd).map((c) => c[1])

let cmp: ReturnType<typeof mount> | undefined
let onClose: ReturnType<typeof vi.fn<() => void>>
function show(myCustomStatus: MyCustomStatus = { current: null, until: null, recent: [] }) {
  app.state = { ...emptyState(), me: alex, myCustomStatus }
  onClose = vi.fn<() => void>()
  cmp = mount(CustomStatusDialog, { target: document.body, props: { onClose } })
  flushSync()
}
const input = () => document.querySelector<HTMLInputElement>('input[aria-label="Custom status"]')!
const select = () => document.querySelector<HTMLSelectElement>('select')!
const button = (t: string) => [...document.querySelectorAll<HTMLButtonElement>('button')].find((b) => b.textContent?.trim() === t)
const type = (v: string) => { input().value = v; input().dispatchEvent(new Event('input', { bubbles: true })); flushSync() }
const choose = (v: string) => { select().value = v; select().dispatchEvent(new Event('change', { bubbles: true })); flushSync() }
const submit = async () => { document.querySelector('form')!.dispatchEvent(new Event('submit', { cancelable: true })); await tick(); await tick(); flushSync() }

beforeEach(() => {
  vi.useFakeTimers({ toFake: ['Date'] })
  vi.setSystemTime(NOW)
  clock.nowIso = NOW.toISOString()
  vi.mocked(invoke).mockReset()
  vi.mocked(invoke).mockImplementation((async () => undefined) as never)
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
  vi.useRealTimers()
})

describe('CustomStatusDialog (#152)', () => {
  it('a new one: clears at the end of today by default, the text sent as the server keeps it', async () => {
    show()
    expect(document.querySelector('[role=dialog]')?.getAttribute('aria-label')).toBe('Set a custom status')
    expect([...select().options].map((o) => o.textContent)).toEqual(['30 minutes', '1 hour', '4 hours', 'Today', 'This week', "Don't clear"])
    expect(select().value).toBe('today')
    expect(button('Save')!.disabled).toBe(true)
    expect(button('Clear status')).toBeUndefined()
    type('  Gaming \n now ')
    await submit()
    expect(calls('set_custom_status')).toEqual([{ text: 'Gaming now', emoji: null, until: endOfToday(NOW).toISOString() }])
    expect(onClose).toHaveBeenCalled()
  })

  it('each clear-after sends its end; never sends none', async () => {
    show()
    type('Lunch')
    choose('30m')
    await submit()
    choose('never')
    await submit()
    expect(calls('set_custom_status').map((a) => (a as { until: string | null }).until)).toEqual([new Date(NOW.getTime() + 30 * 60_000).toISOString(), null])
  })

  it('editing keeps its end unless another is picked; Clear status clears it', async () => {
    const until = new Date(NOW.getTime() + 3_600_000).toISOString()
    show({ current: { text: 'Gaming', emoji: '🎮' }, until, recent: [] })
    expect(document.querySelector('[role=dialog]')?.getAttribute('aria-label')).toBe('Edit your custom status')
    expect(input().value).toBe('Gaming')
    expect(document.querySelector('.emoji-btn img.status-emoji')?.getAttribute('alt')).toBe('🎮')
    expect(select().value).toBe('keep')
    type('Still gaming')
    await submit()
    expect(calls('set_custom_status')).toEqual([{ text: 'Still gaming', emoji: '🎮', until }])
    button('Clear status')!.click()
    await tick()
    expect(calls('clear_custom_status')).toHaveLength(1)
  })

  it('a recent one fills the form (yours now is not offered again)', () => {
    show({ current: { text: 'Gaming', emoji: null }, until: null, recent: [{ text: 'Gaming', emoji: null }, { text: 'Lunch', emoji: '🍕' }] })
    const recent = [...document.querySelectorAll<HTMLButtonElement>('.recent button')]
    expect(recent.map((b) => b.getAttribute('aria-label'))).toEqual(['Use Lunch'])
    recent[0].click()
    flushSync()
    expect(input().value).toBe('Lunch')
    expect(document.querySelector('.emoji-btn img.status-emoji')?.getAttribute('alt')).toBe('🍕')
  })

  it('counts characters as the server does: 128 fit, 129 do not', async () => {
    show()
    type('👍'.repeat(128))
    expect(button('Save')!.disabled).toBe(false)
    type('👍'.repeat(129))
    expect(document.querySelector('[role=alert]')?.textContent).toContain('128')
    expect(button('Save')!.disabled).toBe(true)
    await submit()
    expect(calls('set_custom_status')).toEqual([])
  })

  it('a refusal says why and stays open', async () => {
    vi.mocked(invoke).mockImplementation((async () => { throw { code: 'bad_request', message: 'unknown emoji' } }) as never)
    show()
    type('hi')
    await submit()
    await tick()
    flushSync()
    expect(document.querySelector('[role=alert]')?.textContent).toContain('unknown emoji')
    expect(onClose).not.toHaveBeenCalled()
  })
})
