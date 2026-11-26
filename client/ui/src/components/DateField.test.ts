// @vitest-environment jsdom
// The day field Plans, Send later and Scheduled share (#643): a button showing the day, opening the shared DatePicker.
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { fakeMatchMedia } from '../lib/testMotion'

vi.stubGlobal('matchMedia', fakeMatchMedia(false).matchMedia)

import DateFieldHarness from './DateFieldHarness.test.svelte'

let cmp: ReturnType<typeof mount> | undefined
const seen = vi.fn()

beforeEach(() => {
  vi.useFakeTimers({ toFake: ['Date'] })
  vi.setSystemTime(new Date(2026, 10, 16, 10, 0))
  seen.mockReset()
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
  vi.useRealTimers()
})

function show(props: Record<string, unknown> = {}) {
  cmp = mount(DateFieldHarness, { target: document.body, props: { value: '2026-11-20', onChange: seen, ...props } })
  flushSync()
}
const button = () => document.querySelector<HTMLButtonElement>('button.date-field')!
const picker = () => document.querySelector<HTMLElement>('.date-picker')
const cell = (d: string) => picker()!.querySelector<HTMLButtonElement>(`[data-day="${d}"]`)!
const key = (el: Element, k: string) => {
  const e = new KeyboardEvent('keydown', { key: k, bubbles: true, cancelable: true })
  el.dispatchEvent(e)
  flushSync()
  return e
}

describe('DateField (#643)', () => {
  it('a button showing the day, not a native date input', () => {
    show()
    expect(document.querySelector('input[type=date]')).toBeNull()
    expect(button().textContent).toContain('20 Nov 2026')
    expect(button().getAttribute('aria-label')).toBe('Day: 20 Nov 2026')
    expect(button().getAttribute('aria-haspopup')).toBe('dialog')
    expect(button().getAttribute('aria-expanded')).toBe('false')
  })

  it('opens the shared picker on its day; a pick sets the same YYYY-MM-DD a native input wrote', () => {
    show()
    button().click()
    flushSync()
    expect(picker()).not.toBeNull()
    expect(button().getAttribute('aria-expanded')).toBe('true')
    expect(cell('2026-11-20').getAttribute('aria-selected')).toBe('true')
    cell('2026-11-27').click()
    flushSync()
    expect(seen).toHaveBeenLastCalledWith('2026-11-27')
    expect(picker()).toBeNull()
    expect(button().textContent).toContain('27 Nov 2026')
    expect(document.activeElement).toBe(button())
  })

  it('days before min are disabled and can\'t be picked', () => {
    show({ min: '2026-11-16' })
    button().click()
    flushSync()
    expect(cell('2026-11-15').disabled).toBe(true)
    expect(cell('2026-11-16').disabled).toBe(false)
    cell('2026-11-15').click()
    flushSync()
    expect(seen).not.toHaveBeenCalled()
  })

  it('keyboard: Enter opens it without reaching the form, arrows and Enter pick, Escape closes back on the button', () => {
    const formKey = vi.fn()
    show({ onFormKey: formKey })
    button().focus()
    const enter = key(button(), 'Enter')
    expect(formKey).not.toHaveBeenCalled()
    expect(enter.defaultPrevented).toBe(false) // the browser's own Enter-clicks-the-button still happens
    button().click() // what that default does
    flushSync()
    expect(picker()).not.toBeNull()
    expect((document.activeElement as HTMLElement).dataset.day).toBe('2026-11-20')
    key(document.activeElement!, 'ArrowRight')
    key(document.activeElement!, 'Enter')
    expect(seen).toHaveBeenLastCalledWith('2026-11-21')
    expect(picker()).toBeNull()
    button().click()
    flushSync()
    key(document.activeElement!, 'Escape')
    expect(picker()).toBeNull()
    expect(document.activeElement).toBe(button())
    expect(formKey).not.toHaveBeenCalled()
  })

  it('other keys on the button still reach its own handler (e.g. Escape cancels an edit)', () => {
    const onkeydown = vi.fn()
    show({ onkeydown })
    key(button(), 'Escape')
    expect(onkeydown).toHaveBeenCalledTimes(1)
  })

  it('an empty value asks for a day', () => {
    show({ value: '' })
    expect(button().textContent).toContain('Pick a day')
  })
})
