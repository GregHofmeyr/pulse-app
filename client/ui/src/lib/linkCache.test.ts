import { afterEach, describe, expect, it, vi } from 'vitest'
import { linkCache, RENEW_MARGIN_MS } from './linkCache'

// A link that lives `ms` from now (counted past the renew margin), as the store would sign it.
const link = (n: number, ms = RENEW_MARGIN_MS + 60_000) => ({ url: `https://files.test/${n}`, expires_at: new Date(Date.now() + ms).toISOString() })

afterEach(() => vi.useRealTimers())

describe('linkCache bounds', () => {
  it('keeps at most `max` links, dropping the least recently handed out', async () => {
    const cache = linkCache(3)
    for (let n = 0; n < 3; n++) cache.put(`k${n}`, link(n), Date.now())
    const fetch = vi.fn(async () => link(99))
    await cache.get('k0', fetch) // k0 used: k1 is now the oldest
    cache.put('k3', link(3), Date.now())
    expect(await cache.get('k0', fetch)).toBe('https://files.test/0')
    expect(await cache.get('k2', fetch)).toBe('https://files.test/2')
    expect(await cache.get('k3', fetch)).toBe('https://files.test/3')
    expect(fetch).not.toHaveBeenCalled()
    expect(await cache.get('k1', fetch)).toBe('https://files.test/99') // dropped: fetched again
  })

  it('drops run-out links as new ones are stored', async () => {
    vi.useFakeTimers()
    const cache = linkCache(1000)
    cache.put('old', link(1), Date.now())
    vi.advanceTimersByTime(120_000) // past its renew point
    cache.put('new', link(2), Date.now())
    const fetch = vi.fn(async () => link(3))
    expect(await cache.get('old', fetch)).toBe('https://files.test/3')
    expect(await cache.get('new', fetch)).toBe('https://files.test/2')
    expect(fetch).toHaveBeenCalledTimes(1)
  })
})
