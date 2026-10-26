import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { FRESH_MS, HOVER_DELAY_MS, hoverPrefetch } from './hoverPrefetch'

describe('hoverPrefetch', () => {
  beforeEach(() => vi.useFakeTimers())
  afterEach(() => vi.useRealTimers())

  const setup = () => {
    const fetch = vi.fn((_id: string) => Promise.resolve())
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

  it('does not ask again for a channel prefetched while that token is still fresh', async () => {
    const { fetch, p } = setup()
    p.enter('a')
    await vi.advanceTimersByTimeAsync(HOVER_DELAY_MS)
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

  it('a click cancels the pending prefetch (the join fetches its own token)', () => {
    const { fetch, p } = setup()
    p.enter('a')
    vi.advanceTimersByTime(150)
    p.cancel() // the row was clicked
    vi.advanceTimersByTime(1000)
    expect(fetch).not.toHaveBeenCalled()
  })

  it('a failed prefetch is not remembered: the next rest on the channel asks again', async () => {
    const fetch = vi.fn((_id: string) => Promise.reject(new Error('offline')))
    const p = hoverPrefetch(fetch, { now: () => Date.now() })
    p.enter('a')
    await vi.advanceTimersByTimeAsync(HOVER_DELAY_MS)
    p.leave('a')
    p.enter('a')
    await vi.advanceTimersByTimeAsync(HOVER_DELAY_MS)
    expect(fetch).toHaveBeenCalledTimes(2)
  })

  it('a prefetch still in flight is not asked for again', async () => {
    let done!: () => void
    const fetch = vi.fn((_id: string) => new Promise<void>((r) => (done = r)))
    const p = hoverPrefetch(fetch, { now: () => Date.now() })
    p.enter('a')
    await vi.advanceTimersByTimeAsync(HOVER_DELAY_MS)
    p.leave('a')
    p.enter('a')
    await vi.advanceTimersByTimeAsync(HOVER_DELAY_MS)
    expect(fetch).toHaveBeenCalledTimes(1)
    done()
    await vi.advanceTimersByTimeAsync(0)
    p.leave('a')
    p.enter('a')
    await vi.advanceTimersByTimeAsync(HOVER_DELAY_MS)
    expect(fetch).toHaveBeenCalledTimes(1)
  })
})
