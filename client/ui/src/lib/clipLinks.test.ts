import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { clipLink, forgetClipLinks, RENEW_MARGIN_MS } from './clipLinks'
import type { ClipUrl } from './protocol/ClipUrl'

const T0 = Date.parse('2026-11-09T10:00:00.000Z')
let n = 0
// A signed link the way the server hands it out: an hour's X-Amz-Expires and the matching expires_at.
const signed = (ttlS = 3600, expires = new Date(Date.now() + ttlS * 1000).toISOString()): ClipUrl =>
  ({ url: `https://store.test/clips/k.mp4?X-Amz-Expires=${ttlS}&X-Amz-Signature=s${++n}`, expires_at: expires })

beforeEach(() => {
  vi.useFakeTimers()
  vi.setSystemTime(T0)
  forgetClipLinks()
  n = 0
})
afterEach(() => vi.useRealTimers())

describe('clipLink', () => {
  it('reuses one link per clip until the safety margin before it expires', async () => {
    const fetch = vi.fn(async () => signed())
    expect(await clipLink('M1', fetch)).toContain('s1')
    vi.setSystemTime(T0 + 3600_000 - RENEW_MARGIN_MS - 1000)
    expect(await clipLink('M1', fetch)).toContain('s1')
    expect(fetch).toHaveBeenCalledTimes(1)
    vi.setSystemTime(T0 + 3600_000 - RENEW_MARGIN_MS)
    expect(await clipLink('M1', fetch)).toContain('s2')
    expect(fetch).toHaveBeenCalledTimes(2)
  })

  it('keeps each clip’s link apart', async () => {
    const fetch = vi.fn(async () => signed())
    await clipLink('M1', fetch)
    await clipLink('M2', fetch)
    expect(await clipLink('M1', fetch)).toContain('s1')
    expect(await clipLink('M2', fetch)).toContain('s2')
    expect(fetch.mock.calls.map((c) => (c as unknown[])[0])).toEqual(['M1', 'M2'])
  })

  it('players asking at once share one request', async () => {
    let resolve!: (v: ClipUrl) => void
    const fetch = vi.fn(() => new Promise<ClipUrl>((r) => { resolve = r }))
    const a = clipLink('M1', fetch)
    const b = clipLink('M1', fetch)
    resolve(signed())
    expect(await a).toBe(await b)
    expect(fetch).toHaveBeenCalledTimes(1)
  })

  it('fresh replaces the stored link (a playback error that may be an expired link)', async () => {
    const fetch = vi.fn(async () => signed())
    await clipLink('M1', fetch)
    expect(await clipLink('M1', fetch, { fresh: true })).toContain('s2')
    expect(await clipLink('M1', fetch)).toContain('s2')
  })

  it('a failed request stores nothing and the next one asks again', async () => {
    const fetch = vi.fn(async () => signed())
    fetch.mockRejectedValueOnce(new Error('offline'))
    await expect(clipLink('M1', fetch)).rejects.toThrow('offline')
    expect(await clipLink('M1', fetch)).toContain('s1')
    expect(fetch).toHaveBeenCalledTimes(2)
  })

  it('counts the lifetime on this device’s clock, so a skewed clock neither reuses a dead link nor never reuses', async () => {
    // Our clock is two hours behind the server's: expires_at looks three hours away, but the link
    // only lives an hour from when we asked.
    const behind = vi.fn(async () => signed(3600, new Date(T0 + 3 * 3600_000).toISOString()))
    await clipLink('M1', behind)
    vi.setSystemTime(T0 + 3600_000 - RENEW_MARGIN_MS)
    expect(await clipLink('M1', behind)).toContain('s2')
    // Two hours ahead: expires_at is already past, yet the link is good for the hour.
    forgetClipLinks()
    vi.setSystemTime(T0)
    const ahead = vi.fn(async () => signed(3600, new Date(T0 - 3600_000).toISOString()))
    const first = await clipLink('M1', ahead)
    expect(await clipLink('M1', ahead)).toBe(first)
  })

  it('without X-Amz-Expires it goes by expires_at, and a link with neither is used once', async () => {
    const fetch = vi.fn(async (): Promise<ClipUrl> => ({ url: `http://127.0.0.1:7895/k?sig=${++n}`, expires_at: new Date(T0 + 600_000).toISOString() }))
    await clipLink('M1', fetch)
    expect(await clipLink('M1', fetch)).toContain('sig=1')
    vi.setSystemTime(T0 + 600_000 - RENEW_MARGIN_MS)
    expect(await clipLink('M1', fetch)).toContain('sig=2')
    const bare = vi.fn(async (): Promise<ClipUrl> => ({ url: `http://127.0.0.1:7895/k?sig=${++n}`, expires_at: '' }))
    const a = await clipLink('M2', bare)
    expect(await clipLink('M2', bare)).not.toBe(a)
  })

  it('signing out forgets every link, including one still being fetched', async () => {
    const fetch = vi.fn(async () => signed())
    await clipLink('M1', fetch)
    let resolve!: (v: ClipUrl) => void
    const slow = vi.fn(() => new Promise<ClipUrl>((r) => { resolve = r }))
    const late = clipLink('M2', slow)
    forgetClipLinks()
    resolve(signed())
    await late // the caller that asked still gets its answer
    expect(await clipLink('M1', fetch)).toContain('s3')
    expect(await clipLink('M2', fetch)).toContain('s4')
  })
})
