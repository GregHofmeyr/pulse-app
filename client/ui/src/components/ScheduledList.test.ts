// @vitest-environment jsdom
// Editing a scheduled message (#299) picks its day with the shared DatePicker (#643).
import { flushSync, mount, tick, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { fakeMatchMedia } from '../lib/testMotion'

vi.stubGlobal('matchMedia', fakeMatchMedia(false).matchMedia)
const editScheduled = vi.fn(async (_id: string, _c: { content?: string; sendAt?: string }) => {})
vi.mock('../lib/scheduled.svelte', () => ({
  editScheduled: (id: string, c: { content?: string; sendAt?: string }) => editScheduled(id, c),
  deleteScheduled: async () => {},
  sendScheduledNow: async () => {},
  whereTo: () => 'general',
}))

import ScheduledList from './ScheduledList.svelte'
import { fieldsToIso } from '../lib/plan'
import type { ScheduledMessage } from '../lib/protocol/ScheduledMessage'

let cmp: ReturnType<typeof mount> | undefined
const ITEM = {
  id: 's1', channel_id: 'c1', content: 'hello', reply_to_id: null, send_at: fieldsToIso('2026-11-18', '09:30')!,
  created_at: '2026-11-16T08:00:00Z', edited_at: null, failed: null,
} as unknown as ScheduledMessage

beforeEach(() => {
  vi.useFakeTimers({ toFake: ['Date'] })
  vi.setSystemTime(new Date(2026, 10, 16, 10, 0))
  editScheduled.mockClear()
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
  vi.useRealTimers()
})

function show() {
  cmp = mount(ScheduledList, { target: document.body, props: { items: [ITEM], empty: 'Nothing' } })
  flushSync()
  ;[...document.querySelectorAll<HTMLButtonElement>('.actions .btn')].find((b) => b.textContent === 'Edit')!.click()
  flushSync()
}
const dayBtn = () => document.querySelector<HTMLButtonElement>('.sched button.date-field')!
const picker = () => document.querySelector<HTMLElement>('.date-picker')
const cell = (d: string) => picker()!.querySelector<HTMLButtonElement>(`[data-day="${d}"]`)!
const save = () => [...document.querySelectorAll<HTMLButtonElement>('.actions .btn')].find((b) => b.textContent === 'Save')!

describe('ScheduledList edit day (#643)', () => {
  it('uses the shared picker, not a native date input; the time input stays', () => {
    show()
    expect(document.querySelector('.sched input[type=date]')).toBeNull()
    expect(dayBtn().textContent).toContain('18 Nov 2026')
    expect(document.querySelector<HTMLInputElement>('.sched input[type=time]')!.value).toBe('09:30')
  })

  it('a picked day saves the same instant the native input gave', async () => {
    show()
    dayBtn().click()
    flushSync()
    cell('2026-11-25').click()
    flushSync()
    save().click()
    await tick()
    expect(editScheduled).toHaveBeenCalledWith('s1', { sendAt: fieldsToIso('2026-11-25', '09:30') })
  })

  it('past days are disabled', () => {
    show()
    dayBtn().click()
    flushSync()
    expect(cell('2026-11-15').disabled).toBe(true)
    expect(cell('2026-11-16').disabled).toBe(false)
  })

  it('Escape in the picker closes it and keeps the edit open; Escape on the day button leaves the edit', () => {
    show()
    dayBtn().click()
    flushSync()
    ;(document.activeElement as HTMLElement).dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }))
    flushSync()
    expect(picker()).toBeNull()
    expect(dayBtn()).not.toBeNull()
    dayBtn().dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }))
    flushSync()
    expect(document.querySelector('.sched button.date-field')).toBeNull()
  })
})
