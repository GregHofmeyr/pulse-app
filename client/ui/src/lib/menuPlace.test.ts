// A cursor menu's place in the window, and the corner its pop-in grows from (#181).
import { describe, expect, it } from 'vitest'
import { placeMenu } from './menuPlace'

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
