import { describe, expect, it } from 'vitest'
import { applyEmoji, emojiQuery, insertAt } from './emojiComplete'

describe('emoji autocomplete', () => {
  it('query_needs_two_chars', () => {
    expect(emojiQuery(':s')).toBeNull()
    expect(emojiQuery(':')).toBeNull()
    expect(emojiQuery(':sm')).toBe('sm')
    expect(emojiQuery('hey :thumbs_up')).toBe('thumbs_up')
    expect(emojiQuery('line\n:po')).toBe('po')
    expect(emojiQuery('http://xy')).toBeNull() // no whitespace before ':'
    expect(emojiQuery('a:sm')).toBeNull()
    expect(emojiQuery(':sm ')).toBeNull()
  })

  it('apply_inserts_token_and_space', () => {
    expect(applyEmoji('hi :sm there', 6, '😄')).toEqual({ text: 'hi 😄  there', caret: 6 })
    expect(applyEmoji(':po', 3, '<:pog:1>')).toEqual({ text: '<:pog:1> ', caret: 9 })
  })

  it('insertAt replaces the selection and adds a space', () => {
    expect(insertAt('ab', 1, 1, '👍')).toEqual({ text: 'a👍 b', caret: 4 })
    expect(insertAt('abcd', 1, 3, 'x')).toEqual({ text: 'ax d', caret: 3 })
  })
})
