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
