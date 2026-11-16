import { describe, expect, it } from 'vitest'
import { lineWithHit, relabelMentions, segments } from './highlight'

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
    expect(lineWithHit('“a” — sam\n“pizza” — jo', [r(11, 16)])).toEqual({ text: '“pizza” — jo', highlights: [r(1, 6)], index: 1 })
  })
  it('is the first line when nothing matched', () => {
    expect(lineWithHit('one\ntwo', [])).toEqual({ text: 'one', highlights: [], index: 0 })
  })
})

describe('relabelMentions (#114)', () => {
  const lookup = (name: string) => ({ sam: 'U2', kim: 'U3' })[name.toLowerCase()] ?? null
  const label = (id: string) => ({ U2: 'Sammy', U3: 'K' })[id] ?? 'unknown'
  it('writes known mentions as their names and moves the highlights after them', () => {
    // "pizza" at 9..14 in "hey @sam pizza"
    expect(relabelMentions('hey @sam pizza', [r(9, 14)], lookup, label)).toEqual({ text: 'hey @Sammy pizza', highlights: [r(11, 16)] })
    // a shorter name moves them back
    expect(relabelMentions('@kim and pizza', [r(9, 14)], lookup, label)).toEqual({ text: '@K and pizza', highlights: [r(7, 12)] })
  })
  it('keeps a highlight that falls in a mention on the whole name written there', () => {
    // a match on "sam" inside "@sam" (5..8)
    expect(relabelMentions('hey @sam pizza', [r(5, 8)], lookup, label)).toEqual({ text: 'hey @Sammy pizza', highlights: [r(4, 10)] })
  })
  it('leaves unknown mentions, and text without any, as they are', () => {
    expect(relabelMentions('hi @jo pizza', [r(7, 12)], lookup, label)).toEqual({ text: 'hi @jo pizza', highlights: [r(7, 12)] })
    expect(relabelMentions('pizza', [r(0, 5)], lookup, label)).toEqual({ text: 'pizza', highlights: [r(0, 5)] })
  })
})
