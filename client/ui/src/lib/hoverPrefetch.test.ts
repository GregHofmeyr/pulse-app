import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { FRESH_MS, HOVER_DELAY_MS, hoverPrefetch } from './hoverPrefetch'

describe('hoverPrefetch', () => {
  beforeEach(() => vi.useFakeTimers())
  afterEach(() => vi.useRealTimers())

  const setup = () => {
    const fetch = vi.fn()
    const p = hoverPrefetch(fetch, { now: () => Date.now() })
    return { fetch, p }
  }

  it('fetches once the pointer rests on a channel', () => {
    const { fetch, p } = setup()
    p.enter('a')
    vi.advanceTimersByTime(HOVER_DELAY_MS - 1)
    expect(fetch).not.toHaveBeenCalled()
    vi.advanceTimersByTime(1)
    expect(fetch).toHaveBeenCalledExactlyOnceWith('a')
  })

  it('a sweep across the list fetches only where it stops', () => {
    const { fetch, p } = setup()
    for (const id of ['a', 'b', 'c', 'd']) {
      p.enter(id)
      vi.advanceTimersByTime(50)
      p.leave(id)
    }
    p.enter('e')
    vi.advanceTimersByTime(HOVER_DELAY_MS)
    expect(fetch.mock.calls).toEqual([['e']])
  })

  it('leaving before the delay fetches nothing', () => {
    const { fetch, p } = setup()
    p.enter('a')
    vi.advanceTimersByTime(100)
    p.leave('a')
    vi.advanceTimersByTime(1000)
    expect(fetch).not.toHaveBeenCalled()
  })

  it('does not ask again for a channel prefetched while that token is still fresh', () => {
    const { fetch, p } = setup()
    p.enter('a')
    vi.advanceTimersByTime(HOVER_DELAY_MS)
    p.leave('a')
    p.enter('a') // hover and focus both fire, or the pointer comes back
    vi.advanceTimersByTime(HOVER_DELAY_MS)
    expect(fetch).toHaveBeenCalledTimes(1)
    vi.advanceTimersByTime(FRESH_MS)
    p.enter('a')
    vi.advanceTimersByTime(HOVER_DELAY_MS)
    expect(fetch).toHaveBeenCalledTimes(2)
  })

  it('a different channel is fetched even right after another one', () => {
    const { fetch, p } = setup()
    p.enter('a')
    vi.advanceTimersByTime(HOVER_DELAY_MS)
    p.enter('b')
    vi.advanceTimersByTime(HOVER_DELAY_MS)
    expect(fetch.mock.calls).toEqual([['a'], ['b']])
  })

  it('entering the same channel twice (hover then focus) starts one timer', () => {
    const { fetch, p } = setup()
    p.enter('a')
    vi.advanceTimersByTime(150)
    p.enter('a')
    vi.advanceTimersByTime(50)
    expect(fetch).toHaveBeenCalledTimes(1)
  })
})
