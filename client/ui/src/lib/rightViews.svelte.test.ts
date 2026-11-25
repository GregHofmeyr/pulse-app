// The right panel's views (#638): which one shows, by the person's choices per place, arrivals, and closes.
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { chooseRightView, claimRightView, forgetRightViews, settleRightViews, shownRightView } from './rightViews.svelte'

const RANK = ['search', 'chat', 'build', 'members', 'group']

beforeEach(() => {
  vi.useRealTimers()
  forgetRightViews()
})

describe('right panel views', () => {
  it('with no choice yet, the first available by rank shows', () => {
    expect(shownRightView('s:S1', ['members'], RANK)).toBe('members')
    expect(shownRightView('s:S1', ['members', 'build'], RANK)).toBe('build')
    expect(shownRightView('s:S1', [], RANK)).toBeNull()
  })

  it('a choice is kept per place', () => {
    chooseRightView('s:S1', 'members')
    expect(shownRightView('s:S1', ['build', 'members'], RANK)).toBe('members')
    expect(shownRightView('s:S2', ['build', 'members'], RANK)).toBe('build')
  })

  it('a view that arrives takes the column in the place it arrives in', () => {
    chooseRightView('s:S1', 'members')
    claimRightView('build')
    settleRightViews('s:S1', ['build', 'members'])
    expect(shownRightView('s:S1', ['build', 'members'], RANK)).toBe('build')
  })

  it('a claim waits until its view is there, but not for long', () => {
    vi.useFakeTimers()
    chooseRightView('s:S1', 'members')
    claimRightView('chat')
    settleRightViews('s:S1', ['members'])
    settleRightViews('s:S1', ['chat', 'members'])
    expect(shownRightView('s:S1', ['chat', 'members'], RANK)).toBe('chat')
    chooseRightView('s:S1', 'members')
    claimRightView('search')
    vi.advanceTimersByTime(5000)
    settleRightViews('s:S1', ['search', 'members'])
    expect(shownRightView('s:S1', ['search', 'members'], RANK)).toBe('members')
  })

  it('closing the one showing goes back to the one before', () => {
    chooseRightView('s:S1', 'members')
    chooseRightView('s:S1', 'build')
    chooseRightView('s:S1', 'search')
    expect(shownRightView('s:S1', ['members', 'build'], RANK)).toBe('build')
    expect(shownRightView('s:S1', ['members'], RANK)).toBe('members')
  })
})
