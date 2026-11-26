// @vitest-environment jsdom
// Send later's "Pick a date and time…" (#299) picks its day with the shared DatePicker (#643).
import { flushSync, mount, tick, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { fakeMatchMedia } from '../lib/testMotion'

vi.stubGlobal('matchMedia', fakeMatchMedia(false).matchMedia)

import WhenPicker from './WhenPicker.svelte'
import { fieldsToIso } from '../lib/plan'

let cmp: ReturnType<typeof mount> | undefined
const onPick = vi.fn(async (_at: string) => {})
const onClose = vi.fn()

beforeEach(() => {
  vi.useFakeTimers({ toFake: ['Date'] })
  vi.setSystemTime(new Date(2026, 10, 16, 10, 0))
  onPick.mockClear()
  onClose.mockClear()
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
  vi.useRealTimers()
})

async function show() {
  const anchor = document.createElement('button')
  document.body.append(anchor)
  cmp = mount(WhenPicker, { target: document.body, props: { anchor, title: 'Send later', confirm: 'Schedule', onPick, onClose } })
  flushSync()
  await new Promise((r) => setTimeout(r, 0)) // clickOutside arms
}
const dayBtn = () => document.querySelector<HTMLButtonElement>('.when button.date-field')!
const picker = () => document.querySelector<HTMLElement>('.date-picker')
const cell = (d: string) => picker()!.querySelector<HTMLButtonElement>(`[data-day="${d}"]`)!
const time = () => document.querySelector<HTMLInputElement>('.when input[type=time]')!
const schedule = () => [...document.querySelectorAll<HTMLButtonElement>('.when .btn')].find((b) => b.textContent === 'Schedule')!

describe('WhenPicker day (#643)', () => {
  it('uses the shared picker, not a native date input; the time input stays', async () => {
    await show()
    expect(document.querySelector('.when input[type=date]')).toBeNull()
    expect(dayBtn().textContent).toContain('16 Nov 2026') // the default: the next whole hour after an hour from now
    expect(time().value).toBe('12:00')
  })

  it('a picked day schedules the same instant the native input gave', async () => {
    await show()
    dayBtn().click()
    flushSync()
    const day = cell('2026-11-20')
    day.dispatchEvent(new Event('pointerdown', { bubbles: true })) // a press in the picker isn't outside the form
    day.click()
    flushSync()
    expect(onClose).not.toHaveBeenCalled()
    expect(picker()).toBeNull()
    schedule().click()
    await tick()
    expect(onPick).toHaveBeenCalledWith(fieldsToIso('2026-11-20', '12:00'))
  })

  it('past days are disabled', async () => {
    await show()
    dayBtn().click()
    flushSync()
    expect(cell('2026-11-15').disabled).toBe(true)
    expect(cell('2026-11-16').disabled).toBe(false)
    expect(picker()!.querySelector<HTMLButtonElement>('[aria-label="Previous month"]')!.disabled).toBe(true)
  })

  it('Escape closes only the picker, then the form', async () => {
    await show()
    dayBtn().click()
    flushSync()
    ;(document.activeElement as HTMLElement).dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }))
    flushSync()
    expect(picker()).toBeNull()
    expect(onClose).not.toHaveBeenCalled()
    expect(document.activeElement).toBe(dayBtn())
    dayBtn().dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }))
    expect(onClose).toHaveBeenCalled()
  })

  it('Enter on the day opens the picker instead of scheduling', async () => {
    await show()
    dayBtn().dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true }))
    await tick()
    expect(onPick).not.toHaveBeenCalled()
  })
})
