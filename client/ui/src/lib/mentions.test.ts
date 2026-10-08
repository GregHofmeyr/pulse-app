import { describe, expect, it } from 'vitest'
import { applyMention, mentionQuery, suggest } from './mentions'

const people = [
  { id: 'U1', name: 'sam' },
  { id: 'U2', name: 'Sasha' },
  { id: 'U3', name: 'jo' },
]

describe('mention autocomplete', () => {
  it('detects an @token right before the caret', () => {
    expect(mentionQuery('hey @sa')).toBe('sa')
    expect(mentionQuery('@')).toBe('')
    expect(mentionQuery('email a@b')).toBeNull()
    expect(mentionQuery('hey @sam ')).toBeNull()
  })

  it('suggests by case-insensitive prefix, up to 6', () => {
    expect(suggest(people, 'SA').map((p) => p.id)).toEqual(['U1', 'U2'])
    expect(suggest(people, '')).toHaveLength(3)
  })

  it('replaces the partial token and puts the caret after a trailing space', () => {
    expect(applyMention('hey @sa', 7, 'sam')).toEqual({ text: 'hey @sam ', caret: 9 })
    expect(applyMention('@j and more', 2, 'jo')).toEqual({ text: '@jo  and more', caret: 4 })
  })
})
