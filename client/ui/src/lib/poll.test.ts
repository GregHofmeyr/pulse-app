import { describe, expect, it } from 'vitest'
import { nextChoices, parsePollArgs, percent, pollReady, pollStatus, totalLabel, winners } from './poll'
import type { Poll } from './protocol/Poll'

const poll = (over: Partial<Poll> = {}): Poll => ({
  question: 'Q',
  options: ['a', 'b', 'c'],
  multi: false,
  anonymous: false,
  ends_at: null,
  closed_at: null,
  closed: false,
  voters: 0,
  mine: [],
  results: null,
  ...over,
})

describe('/poll arguments', () => {
  it('reads the question up to the ? and the options after it', () => {
    expect(parsePollArgs('')).toEqual({ question: '', options: [] })
    expect(parsePollArgs('Where to eat? Pizza, Sushi ,, Tacos')).toEqual({ question: 'Where to eat?', options: ['Pizza', 'Sushi', 'Tacos'] })
    expect(parsePollArgs('  What now?')).toEqual({ question: 'What now?', options: [] })
  })
  it('takes the whole text as the question when there is no ?', () => {
    expect(parsePollArgs('a, b')).toEqual({ question: 'a, b', options: [] })
  })
})

describe('a poll ready to post', () => {
  it('needs a question and two different options', () => {
    expect(pollReady('Q', ['a', 'b'])).toBe(true)
    expect(pollReady(' ', ['a', 'b'])).toBe(false)
    expect(pollReady('Q', ['a', ' '])).toBe(false)
    expect(pollReady('Q', ['a', ' ', 'b'])).toBe(false)
    expect(pollReady('Q', ['Pizza', 'pizza '])).toBe(false)
    expect(pollReady('q'.repeat(201), ['a', 'b'])).toBe(false)
    expect(pollReady('Q', ['a', 'o'.repeat(81)])).toBe(false)
  })
})

describe('voting', () => {
  it('single choice moves or clears, multiple choice toggles', () => {
    expect(nextChoices([], 1, false)).toEqual([1])
    expect(nextChoices([1], 2, false)).toEqual([2])
    expect(nextChoices([1], 1, false)).toEqual([])
    expect(nextChoices([0], 2, true)).toEqual([0, 2])
    expect(nextChoices([0, 2], 0, true)).toEqual([2])
  })
  it('percent of the people who voted', () => {
    expect(percent(1, 3)).toBe(33)
    expect(percent(0, 0)).toBe(0)
    expect(percent(2, 2)).toBe(100)
  })
  it('winners: the most votes, all of a tie, none without votes', () => {
    const t = (v: number[]) => v.map((votes) => ({ votes, voters: [] }))
    expect(winners(poll({ results: t([1, 3, 2]) }))).toEqual([1])
    expect(winners(poll({ results: t([2, 2, 0]) }))).toEqual([0, 1])
    expect(winners(poll({ results: t([0, 0, 0]) }))).toEqual([])
    expect(winners(poll())).toEqual([])
  })
})

describe('the status line', () => {
  const now = Date.parse('2026-11-09T12:00:00Z')
  it('says the kind, anonymous and the time left', () => {
    expect(pollStatus(poll({ ends_at: '2026-11-09T15:00:00Z' }), now)).toBe('One choice · 3 hours left')
    expect(pollStatus(poll({ multi: true, anonymous: true }), now)).toBe('Multiple choice · Anonymous · No end')
  })
  it('closed, once over, by time too', () => {
    expect(pollStatus(poll({ closed: true }), now)).toBe('One choice · Closed')
    expect(pollStatus(poll({ ends_at: '2026-11-09T11:00:00Z' }), now)).toBe('One choice · Closed')
  })
  it('counts people', () => {
    expect(totalLabel(poll({ voters: 1 }))).toBe('1 vote')
    expect(totalLabel(poll({ voters: 4 }))).toBe('4 votes')
    expect(totalLabel(poll({ voters: 1, multi: true }))).toBe('1 voter')
    expect(totalLabel(poll({ voters: 0, multi: true }))).toBe('0 voters')
  })
})
