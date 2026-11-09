// A cursor menu's place in the window, and the corner its pop-in grows from (#181).
import { describe, expect, it } from 'vitest'
import { placeFlyout, placeMenu } from './menuPlace'

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
