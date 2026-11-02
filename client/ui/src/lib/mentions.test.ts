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
  it('lists everyone (alphabetically) for an empty query, up to the cap', () => {
    expect(rankNames(many, '', 3).map((p) => p.name)).toEqual(['alex', 'Isam', 'jo'])
  })
})
