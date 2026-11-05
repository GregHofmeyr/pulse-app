// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from 'vitest'
import { installTooltips } from './tooltip'

let off: (() => void) | undefined
afterEach(() => {
  off?.()
  document.body.innerHTML = ''
})

const wait = () => new Promise((r) => setTimeout(r, 0))

// #159: across a message's quick actions the tip flashed: each crossing hid it and waited the full
// delay again. An open tip now hands over to the next button with no fade.
describe('installTooltips: handing over between buttons', () => {
  const bar = () => {
    const mk = (label: string) => {
      const b = document.createElement('button')
      b.setAttribute('aria-label', label)
      document.body.append(b)
      return b
    }
    return [mk('Add reaction'), mk('Reply'), mk('Edit')]
  }
  const over = (to: Element, from: Element | null) => {
    if (from) from.dispatchEvent(new MouseEvent('mouseout', { bubbles: true, relatedTarget: to }))
    to.dispatchEvent(new MouseEvent('mouseover', { bubbles: true, relatedTarget: from }))
  }
  const out = (from: Element) => from.dispatchEvent(new MouseEvent('mouseout', { bubbles: true, relatedTarget: document.body }))
  const tip = () => document.querySelector('.pulse-tooltip')!
  /** Every time the tip loses `show` (each one is a fade out). */
  let hides = 0
  const watch = () => {
    hides = 0
    let was = false
    new MutationObserver(() => {
      const now = tip().classList.contains('show')
      if (was && !now) hides++
      was = now
    }).observe(tip(), { attributes: true, attributeFilter: ['class'] })
  }

  it('moving to the next button switches the open tip at once, without hiding it', async () => {
    vi.useFakeTimers()
    try {
      off = installTooltips()
      watch()
      const [react, reply, edit] = bar()
      over(react, null)
      vi.advanceTimersByTime(400)
      expect(tip().textContent).toBe('Add reaction')
      over(reply, react)
      expect(tip().classList.contains('show')).toBe(true)
      expect(tip().textContent).toBe('Reply')
      // back and forth across a seam: always the button under the cursor, never a fade
      over(edit, reply)
      over(reply, edit)
      over(edit, reply)
      vi.advanceTimersByTime(50)
      await Promise.resolve()
      expect(tip().textContent).toBe('Edit')
      expect(hides).toBe(0)
    } finally {
      vi.useRealTimers()
    }
  })

  it('leaving the buttons hides it after a moment; coming straight back shows at once', async () => {
    vi.useFakeTimers()
    try {
      off = installTooltips()
      const [react, reply] = bar()
      over(react, null)
      vi.advanceTimersByTime(400)
      out(react)
      expect(tip().classList.contains('show')).toBe(true) // the grace for a handover
      vi.advanceTimersByTime(200)
      expect(tip().classList.contains('show')).toBe(false)
      // straight back: no full delay
      over(reply, null)
      vi.advanceTimersByTime(0)
      expect(tip().classList.contains('show')).toBe(true)
      expect(tip().textContent).toBe('Reply')
      // after a while away, the full delay again
      out(reply)
      vi.advanceTimersByTime(2000)
      over(react, null)
      vi.advanceTimersByTime(100)
      expect(tip().classList.contains('show')).toBe(false)
      vi.advanceTimersByTime(300)
      expect(tip().classList.contains('show')).toBe(true)
    } finally {
      vi.useRealTimers()
    }
  })

  it('a mouse press still hides it at once', () => {
    vi.useFakeTimers()
    try {
      off = installTooltips()
      const [react] = bar()
      over(react, null)
      vi.advanceTimersByTime(400)
      react.dispatchEvent(new MouseEvent('mousedown', { bubbles: true }))
      expect(tip().classList.contains('show')).toBe(false)
    } finally {
      vi.useRealTimers()
    }
  })
})

describe('installTooltips', () => {
  // #103: clicking ⋯ on a clip popped its tip up with the menu. A mouse press focuses the button
  // after hiding the tip, and that focus showed it again at once.
  it('a mouse click does not show the tip through the focus it gives the button', async () => {
    off = installTooltips()
    const b = document.createElement('button')
    b.setAttribute('aria-label', 'Clip actions')
    document.body.append(b)
    const tip = document.querySelector('.pulse-tooltip')!
    b.dispatchEvent(new MouseEvent('mousedown', { bubbles: true }))
    b.focus() // what the press does next
    b.dispatchEvent(new MouseEvent('mouseup', { bubbles: true }))
    b.click()
    await wait()
    await wait()
    expect(tip.classList.contains('show')).toBe(false)
    // keyboard focus afterwards still shows it at once
    b.blur()
    b.focus()
    await wait()
    expect(tip.classList.contains('show')).toBe(true)
  })

  it('a toggle pressed from the keyboard updates its showing tip (Show … becomes Hide …)', async () => {
    off = installTooltips()
    const b = document.createElement('button')
    b.setAttribute('aria-label', 'Members')
    b.dataset.tip = 'Show members'
    // what a toggle does: its label flips once the click is handled (a framework updates it a tick later)
    b.addEventListener('click', () => queueMicrotask(() => (b.dataset.tip = 'Hide members')))
    document.body.append(b)
    b.focus() // focusin: shown at once
    await wait()
    const tip = document.querySelector('.pulse-tooltip')!
    expect(tip.classList.contains('show')).toBe(true)
    expect(tip.textContent).toBe('Show members')
    b.click() // Enter or Space on a focused button
    await wait()
    await wait()
    expect(tip.classList.contains('show')).toBe(true)
    expect(tip.textContent).toBe('Hide members')
  })
})
