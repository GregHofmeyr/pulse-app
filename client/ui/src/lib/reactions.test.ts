import { describe, expect, it } from 'vitest'
import { optimistic, popoverStyle, tooltipText } from './reactions'

const names = (n: number) => Array.from({ length: n }, (_, i) => String.fromCharCode(65 + i))

describe('tooltipText', () => {
  it('joins three names with "and"', () => expect(tooltipText(['A', 'B', 'C'], 3)).toBe('A, B and C'))
  it('handles one and two names', () => {
    expect(tooltipText(['A'], 1)).toBe('A')
    expect(tooltipText(['A', 'B'], 2)).toBe('A and B')
  })
  it('summarises past ten names', () => expect(tooltipText(names(13), 13)).toBe('A, B, C, D, E, F, G, H, I, J and 3 others'))
  it('counts people the list did not name', () => expect(tooltipText(['A', 'B'], 5)).toBe('A, B and 3 others'))
})

describe('optimistic', () => {
  it('one per person replaces my other reaction optimistically', () => {
    const list = [{ emoji: 'a', count: 2, me: true }, { emoji: 'b', count: 1, me: true }, { emoji: 'c', count: 4, me: false }]
    expect(optimistic(list, 'c', true, 'U1', true)).toEqual([{ emoji: 'a', count: 1, me: false }, { emoji: 'c', count: 5, me: true }])
    // a plain channel keeps them all
    expect(optimistic(list, 'c', true, 'U1')).toHaveLength(3)
    // removing is unaffected
    expect(optimistic(list, 'a', false, 'U1', true)).toEqual([{ emoji: 'a', count: 1, me: false }, { emoji: 'b', count: 1, me: true }, { emoji: 'c', count: 4, me: false }])
    // already mine: nothing changes
    expect(optimistic(list, 'a', true, 'U1', true)).toEqual([{ emoji: 'a', count: 2, me: true }, { emoji: 'c', count: 4, me: false }])
  })

  const base = [{ emoji: '👍', count: 2, me: false }]
  it('add then remove returns to the original', () => {
    const added = optimistic(base, '👍', true, 'U1')
    expect(added).toEqual([{ emoji: '👍', count: 3, me: true }])
    expect(optimistic(added, '👍', false, 'U1')).toEqual(base)
  })
  it('a new emoji appends and removing the last one drops it', () => {
    const added = optimistic([], '🎉', true, 'U1')
    expect(added).toEqual([{ emoji: '🎉', count: 1, me: true }])
    expect(optimistic(added, '🎉', false, 'U1')).toEqual([])
  })
  it('ignores a flip that is already in effect', () => {
    expect(optimistic(base, '👍', false, 'U1')).toBe(base)
  })
})

describe('popoverStyle', () => {
  const size = { width: 360, height: 420 }
  const vp = { width: 1000, height: 800 }
  it('opens below when it fits, above when not', () => {
    expect(popoverStyle({ left: 10, top: 100, bottom: 120, right: 30 }, size, vp)).toContain('top: 126px')
    expect(popoverStyle({ left: 10, top: 700, bottom: 720, right: 30 }, size, vp)).toContain('top: 274px')
  })
  it('stays inside the viewport horizontally', () => {
    expect(popoverStyle({ left: 900, top: 100, bottom: 120, right: 920 }, size, vp)).toContain('left: 634px')
  })
})
