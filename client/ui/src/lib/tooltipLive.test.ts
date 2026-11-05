// @vitest-environment jsdom
import { afterEach, describe, expect, it } from 'vitest'
import { installTooltips } from './tooltip'

let off: (() => void) | undefined
afterEach(() => {
  off?.()
  document.body.innerHTML = ''
})

const wait = () => new Promise((r) => setTimeout(r, 0))

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
