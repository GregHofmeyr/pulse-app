import { describe, expect, it } from 'vitest'
import { applyMention, mentionQuery, rankNames, suggest } from './mentions'

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

  // #47/#48: a person can be found by any of their names; what's written is their username (the
  // composer shows it as a chip with the shown name), never the name typed.
  describe('every name layer', () => {
    const crew = [
      { id: 'U2', name: 'Big Dog', username: 'sam', alts: ['Sam 🎸', 'Big Dog', 'Sammy'] },
      { id: 'U3', name: 'Kim K', username: 'kim', alts: ['Kim K'] },
    ]
    it('matches the username, display name, server nickname and private nickname', () => {
      for (const q of ['sa', 'Sam', 'big', 'dog', 'sammy']) expect(suggest(crew, q).map((p) => p.id), q).toEqual(['U2'])
      expect(suggest(crew, 'k').map((p) => p.id)).toEqual(['U3'])
      expect(suggest(crew, 'x')).toEqual([])
    })
    it('offers the shown name', () => {
      expect(suggest(crew, 'sammy')[0].name).toBe('Big Dog')
    })
  })
})

describe('rankNames (the search box: names you only half remember)', () => {
  const many = [
    { id: 'U1', name: 'Samantha' },
    { id: 'U2', name: 'alex' },
    { id: 'U3', name: 'Isam' },
    { id: 'U4', name: 'sam' },
    { id: 'U5', name: 'jo' },
    { id: 'U6', name: 'Rosamund' },
  ]
  it('matches any part of the name, case-insensitively, prefix matches first', () => {
    expect(rankNames(many, 'SAM').map((p) => p.id)).toEqual(['U4', 'U1', 'U3', 'U6'])
  })
  it('matches on a second name too (a nickname and a username)', () => {
    const named = [{ id: 'U1', name: 'Big Al', alt: 'alex' }, { id: 'U2', name: 'jo' }]
    expect(rankNames(named, 'lex').map((p) => p.id)).toEqual(['U1'])
  })
  it('matches on every name in `alts` too', () => {
    const named = [{ id: 'U1', name: 'Big Al', alts: ['Alexander', 'Sir Al'] }, { id: 'U2', name: 'jo' }]
    expect(rankNames(named, 'xander').map((p) => p.id)).toEqual(['U1'])
    expect(rankNames(named, 'sir').map((p) => p.id)).toEqual(['U1'])
  })
  it('lists everyone (alphabetically) for an empty query, up to the cap', () => {
    expect(rankNames(many, '', 3).map((p) => p.name)).toEqual(['alex', 'Isam', 'jo'])
  })
})
