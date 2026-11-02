import { describe, expect, it } from 'vitest'
import { lineWithHit, segments } from './highlight'

const r = (start: number, end: number) => ({ start, end })

describe('segments', () => {
  it('splits text into plain and matched runs', () => {
    expect(segments('pizza tonight', [r(0, 5)])).toEqual([{ text: 'pizza', hit: true }, { text: ' tonight', hit: false }])
    expect(segments('no hits', [])).toEqual([{ text: 'no hits', hit: false }])
  })
  it('copes with unsorted, overlapping and out-of-range ranges', () => {
    expect(segments('abcdef', [r(4, 99), r(1, 3), r(2, 4)])).toEqual([
      { text: 'a', hit: false },
      { text: 'bcdef', hit: true },
    ])
    expect(segments('ab', [r(5, 9), r(1, 1)])).toEqual([{ text: 'ab', hit: false }])
  })
  it('counts in UTF-16 code units, like the server', () => {
    expect(segments('🔥 clip', [r(3, 7)])).toEqual([{ text: '🔥 ', hit: false }, { text: 'clip', hit: true }])
  })
})

describe('lineWithHit', () => {
  it('is the first line with a match, its ranges moved to it', () => {
    expect(lineWithHit('“a” — sam\n“pizza” — jo', [r(11, 16)])).toEqual({ text: '“pizza” — jo', highlights: [r(1, 6)] })
  })
  it('is the first line when nothing matched', () => {
    expect(lineWithHit('one\ntwo', [])).toEqual({ text: 'one', highlights: [] })
  })
})
