// Drag to reorder (#55): the slot and shift maths the sortable list runs every frame.
import { describe, expect, it } from 'vitest'
import { shiftsFor, slotFor } from './drag'

describe('slotFor', () => {
  const items = [{ top: 0, height: 34 }, { top: 34, height: 34 }, { top: 68, height: 30 }]

  it('counts the items whose middle is above the pointer', () => {
    expect(slotFor(items, -10)).toBe(0)
    expect(slotFor(items, 16)).toBe(0)
    expect(slotFor(items, 18)).toBe(1)
    expect(slotFor(items, 50)).toBe(1)
    expect(slotFor(items, 52)).toBe(2)
    expect(slotFor(items, 84)).toBe(3)
    expect(slotFor(items, 500)).toBe(3)
  })

  it('an empty list has one slot', () => {
    expect(slotFor([], 40)).toBe(0)
  })
})

describe('shiftsFor', () => {
  it('nothing moves while the item is over its own slot', () => {
    expect(shiftsFor(1, 1, 34, 4)).toEqual([0, 0, 0, 0])
  })

  it('moving down: the items it passes move up by its height', () => {
    // item 0 to after item 2 (slot 2 among the other three)
    expect(shiftsFor(0, 2, 34, 4)).toEqual([0, -34, -34, 0])
  })

  it('moving up: the items it passes move down by its height', () => {
    // item 3 to the very top
    expect(shiftsFor(3, 0, 34, 4)).toEqual([34, 34, 34, 0])
  })

  it('coming from another list (fromIndex -1): the items after the slot make room', () => {
    expect(shiftsFor(-1, 1, 30, 3)).toEqual([0, 30, 30])
  })

  it('leaving for another list (toSlot -1): the items after it close the gap', () => {
    expect(shiftsFor(1, -1, 30, 3)).toEqual([0, 0, -30])
  })

  it('the lifted item itself never gets a shift (it follows the pointer)', () => {
    expect(shiftsFor(2, 0, 34, 3)[2]).toBe(0)
  })
})
