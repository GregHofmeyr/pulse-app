// @vitest-environment jsdom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { fakeMatchMedia } from '../lib/testMotion'

vi.stubGlobal('matchMedia', fakeMatchMedia(false).matchMedia)

import DatePicker from './DatePicker.svelte'

let cmp: ReturnType<typeof mount> | undefined
let anchor: HTMLButtonElement
const onPick = vi.fn()
const onClose = vi.fn()

beforeEach(() => {
  onPick.mockReset()
  onClose.mockReset()
  anchor = document.createElement('button')
  anchor.textContent = 'open'
  document.body.appendChild(anchor)
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})

function open(props: Partial<{ value: string | null; max: string | null; min: string | null; today: string }> = {}) {
  cmp = mount(DatePicker, { target: document.body, props: { anchor, label: 'Saved before', today: '2026-11-25', max: '2026-11-25', value: null, onPick, onClose, ...props } })
  flushSync()
}
const dialog = () => document.querySelector<HTMLElement>('.date-picker')!
const cell = (day: string) => dialog().querySelector<HTMLButtonElement>(`[data-day="${day}"]`)!
const heading = () => dialog().querySelector('.month')!.textContent
const focused = () => (document.activeElement as HTMLElement | null)?.dataset.day
const key = (k: string) => {
  ;(document.activeElement as HTMLElement).dispatchEvent(new KeyboardEvent('keydown', { key: k, bubbles: true, cancelable: true }))
  flushSync()
}

describe('DatePicker', () => {
  it('a labelled month grid: weekday headers, 6 weeks, today marked', () => {
    open()
    expect(dialog().getAttribute('role')).toBe('dialog')
    expect(dialog().getAttribute('aria-label')).toBe('Saved before')
    const grid = dialog().querySelector('[role=grid]')!
    expect(grid.getAttribute('aria-labelledby')).toBe(dialog().querySelector('.month')!.id)
    expect(heading()).toBe('November 2026')
    expect([...grid.querySelectorAll('[role=columnheader]')].map((h) => h.textContent)).toEqual(['Mo', 'Tu', 'We', 'Th', 'Fr', 'Sa', 'Su'])
    expect(grid.querySelectorAll('[role=row]')).toHaveLength(7) // the header row and 6 weeks
    expect(grid.querySelectorAll('[role=gridcell]')).toHaveLength(42)
    expect(cell('2026-11-25').getAttribute('aria-current')).toBe('date')
    expect(cell('2026-11-24').hasAttribute('aria-current')).toBe(false)
    expect(cell('2026-11-01').getAttribute('aria-label')).toBe('Sunday 1 November 2026')
  })

  it('future days are disabled, and so is a month after max', () => {
    open()
    expect(cell('2026-11-26').disabled).toBe(true)
    expect(cell('2026-11-25').disabled).toBe(false)
    expect(dialog().querySelector<HTMLButtonElement>('[aria-label="Next month"]')!.disabled).toBe(true)
    cell('2026-11-26').click()
    expect(onPick).not.toHaveBeenCalled()
  })

  it('prev and next move a month', () => {
    open()
    dialog().querySelector<HTMLButtonElement>('[aria-label="Previous month"]')!.click()
    flushSync()
    expect(heading()).toBe('October 2026')
    expect(cell('2026-10-31').disabled).toBe(false)
    dialog().querySelector<HTMLButtonElement>('[aria-label="Next month"]')!.click()
    flushSync()
    expect(heading()).toBe('November 2026')
  })

  it('a click picks the day', () => {
    open()
    cell('2026-11-03').click()
    expect(onPick).toHaveBeenCalledWith('2026-11-03')
  })

  it('opens on the picked day, marked selected', () => {
    open({ value: '2026-09-01' })
    expect(heading()).toBe('September 2026')
    expect(cell('2026-09-01').getAttribute('aria-selected')).toBe('true')
    expect(focused()).toBe('2026-09-01')
  })

  it('arrows move between days (across months), never past max; Enter picks', () => {
    open()
    expect(focused()).toBe('2026-11-25') // nothing picked: today
    key('ArrowRight')
    expect(focused()).toBe('2026-11-25') // tomorrow is in the future
    key('ArrowLeft')
    expect(focused()).toBe('2026-11-24')
    key('ArrowUp')
    expect(focused()).toBe('2026-11-17')
    key('ArrowDown')
    expect(focused()).toBe('2026-11-24')
    for (let i = 0; i < 4; i++) key('ArrowUp')
    expect(focused()).toBe('2026-10-27')
    expect(heading()).toBe('October 2026')
    // only the focused day is in the tab order
    expect(dialog().querySelectorAll('[role=gridcell][tabindex="0"]')).toHaveLength(1)
    key('Enter')
    expect(onPick).toHaveBeenCalledWith('2026-10-27')
  })

  it('Escape closes and gives the focus back to the button', () => {
    open()
    key('Escape')
    expect(onClose).toHaveBeenCalled()
    expect(document.activeElement).toBe(anchor)
  })

  it('placed fixed, inside the window', () => {
    anchor.getBoundingClientRect = () => ({ left: 990, right: 1010, top: 700, bottom: 728, width: 20, height: 28, x: 990, y: 700, toJSON: () => ({}) })
    vi.stubGlobal('innerWidth', 1024)
    vi.stubGlobal('innerHeight', 768)
    open()
    const s = dialog().style
    const left = parseFloat(s.left)
    expect(left).toBeGreaterThanOrEqual(8)
    expect(left).toBeLessThanOrEqual(1024 - 8 - 256) // its 256px width
    expect(s.left && (s.top || s.bottom) && s.maxHeight).toBeTruthy()
    vi.unstubAllGlobals()
    vi.stubGlobal('matchMedia', fakeMatchMedia(false).matchMedia)
  })
})
