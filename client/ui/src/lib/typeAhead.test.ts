import { describe, expect, it } from 'vitest'
import { typeAheadIndex } from './typeAhead'

const labels = ['None (no cards)', '#general', '#games', '#memes', '#sport']

describe('typeAheadIndex (#455)', () => {
  it('jumps to the first label starting with the letter, skipping a leading #', () => {
    expect(typeAheadIndex(labels, 'g', 0)).toBe(1)
    expect(typeAheadIndex(labels, 'S', 0)).toBe(4)
  })
  it('the same letter again cycles on, wrapping', () => {
    expect(typeAheadIndex(labels, 'g', 1)).toBe(2)
    expect(typeAheadIndex(labels, 'g', 2)).toBe(1)
    expect(typeAheadIndex(labels, 'gg', 1)).toBe(2)
  })
  it('typing on narrows the current match', () => {
    expect(typeAheadIndex(labels, 'ga', 1)).toBe(2)
    expect(typeAheadIndex(labels, 'gen', 1)).toBe(1)
  })
  it('skips disabled options; no match is -1', () => {
    expect(typeAheadIndex(labels, 'g', 0, (i) => i === 1)).toBe(2)
    expect(typeAheadIndex(labels, 'z', 0)).toBe(-1)
    expect(typeAheadIndex([], 'a', 0)).toBe(-1)
  })
})
