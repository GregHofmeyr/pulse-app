// A cursor menu's place in the window, and the corner its pop-in grows from (#181).
import { describe, expect, it } from 'vitest'
import { placeFlyout, placeMenu, placePopover } from './menuPlace'

const win = { width: 1000, height: 600 }

describe('placeMenu', () => {
  it('opens at the cursor, growing from its top left', () => {
    expect(placeMenu(100, 80, 220, 200, win)).toEqual({ left: 100, top: 80, origin: 'top left' })
  })

  it('pushed left by the right edge, it grows from its top right (the corner nearest the cursor)', () => {
    expect(placeMenu(950, 80, 220, 200, win)).toEqual({ left: 1000 - 220 - 8, top: 80, origin: 'top right' })
  })

  it('pushed up by the bottom edge, from its bottom left', () => {
    expect(placeMenu(100, 550, 220, 200, win)).toEqual({ left: 100, top: 600 - 200 - 8, origin: 'bottom left' })
  })

  it('in the bottom right corner, from its bottom right', () => {
    expect(placeMenu(990, 590, 220, 200, win).origin).toBe('bottom right')
  })

  it('never off the top or left, even when it is bigger than the room', () => {
    const p = placeMenu(4, 4, 220, 700, win)
    expect(p.left).toBe(8)
    expect(p.top).toBe(8)
    expect(p.origin).toBe('top left')
  })
})

// A menu's flyout (#131, #134): to the left of the menu, or to the right when the left has no room; in a window
// with room on neither side, the roomier side, pulled back over the menu so it stays inside the window.
describe('placeFlyout', () => {
  const w = 1200
  it('to the left when it fits there', () => {
    expect(placeFlyout(600, 230, w)).toEqual({ side: 'left', shift: 0 })
    // 204 = the 4px gap plus the 200px flyout, and 8px kept from the edge
    expect(placeFlyout(212, 230, w)).toEqual({ side: 'left', shift: 0 })
  })
  it('to the right when the left has no room', () => {
    expect(placeFlyout(211, 230, w)).toEqual({ side: 'right', shift: 0 })
    expect(placeFlyout(40, 230, w)).toEqual({ side: 'right', shift: 0 })
  })
  it('room on neither side: the roomier one, pulled back inside the window', () => {
    // a 400px window, the menu at 40..270: 32 to its left, 122 to its right
    expect(placeFlyout(40, 230, 400)).toEqual({ side: 'right', shift: 204 - 122 })
    // the menu at 150..380: 142 to its left, 12 to its right
    expect(placeFlyout(150, 230, 400)).toEqual({ side: 'left', shift: 204 - 142 })
  })
})

describe('placePopover (#454)', () => {
  const bin = { left: 600, right: 630, top: 100, bottom: 128 }
  it('below the button when it fits, its end edge lined up with the button', () => {
    expect(placePopover(bin, { width: 240, height: 120 }, win, 'end')).toEqual({ left: 390, below: true, top: 134, maxHeight: 600 - 128 - 6 - 8 })
  })
  it('start-aligned by default', () => {
    expect(placePopover(bin, { width: 240, height: 120 }, win).left).toBe(600)
  })
  it('above when there is no room below, pinned by its bottom', () => {
    const low = { left: 600, right: 630, top: 560, bottom: 588 }
    expect(placePopover(low, { width: 240, height: 120 }, win, 'end')).toEqual({ left: 390, below: false, bottom: 600 - 560 + 6, maxHeight: 560 - 6 - 8 })
  })
  it('too tall for either side: the roomier side, capped to its room so it scrolls instead of leaving the window', () => {
    const p = placePopover({ left: 10, right: 40, top: 200, bottom: 228 }, { width: 240, height: 900 }, win)
    expect(p.below).toBe(true)
    expect(p.maxHeight).toBe(600 - 228 - 6 - 8)
    expect(p.top! + p.maxHeight).toBeLessThanOrEqual(600 - 8)
  })
  it('kept 8px inside the window across, at both edges', () => {
    expect(placePopover({ left: 2, right: 20, top: 10, bottom: 30 }, { width: 240, height: 50 }, win, 'end').left).toBe(8)
    expect(placePopover({ left: 980, right: 998, top: 10, bottom: 30 }, { width: 240, height: 50 }, win).left).toBe(1000 - 240 - 8)
  })
  it('prefer above (#455): above when it fits there, else below', () => {
    expect(placePopover(bin, { width: 200, height: 80 }, win, 'start', 4, 'above').below).toBe(false)
    expect(placePopover(bin, { width: 200, height: 200 }, win, 'start', 4, 'above').below).toBe(true)
  })
})
