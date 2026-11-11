import { afterEach, describe, expect, it, vi } from 'vitest'
import { attachmentLink, forgetAttachmentLinks, NoLink } from './attachmentLinks'
import { RENEW_MARGIN_MS } from './linkCache'
import type { AttachmentUrl } from './protocol/AttachmentUrl'

const signed = (id: string, n: number): AttachmentUrl => ({
  id, url: `https://store.test/attachments/${id}?X-Amz-Expires=3600&sig=${n}`, expires_at: new Date(Date.now() + 3_600_000).toISOString(),
})

afterEach(() => {
  forgetAttachmentLinks()
  vi.useRealTimers()
})

describe('attachmentLink', () => {
  it('one request signs every file of a message, and each link is reused', async () => {
    let n = 0
    const fetch = vi.fn(async () => (n++, [signed('a', n), signed('b', n)]))
    const [a, b] = await Promise.all([attachmentLink('M1', 'a', fetch), attachmentLink('M1', 'b', fetch)])
    expect(fetch).toHaveBeenCalledTimes(1)
    expect(a).toContain('/a?')
    expect(b).toContain('/b?')
    expect(await attachmentLink('M1', 'b', fetch)).toBe(b)
    expect(fetch).toHaveBeenCalledTimes(1)
  })

  it('asks again shortly before a link expires, and when told to', async () => {
    vi.useFakeTimers()
    let n = 0
    const fetch = vi.fn(async () => [signed('a', ++n)])
    const first = await attachmentLink('M1', 'a', fetch)
    vi.advanceTimersByTime(3_600_000 - RENEW_MARGIN_MS - 1000)
    expect(await attachmentLink('M1', 'a', fetch)).toBe(first)
    vi.advanceTimersByTime(2000)
    const second = await attachmentLink('M1', 'a', fetch)
    expect(second).not.toBe(first)
    const third = await attachmentLink('M1', 'a', fetch, { fresh: true })
    expect(third).not.toBe(second)
    expect(fetch).toHaveBeenCalledTimes(3)
  })

  it('an expired file has no link', async () => {
    const fetch = async () => [{ id: 'a', url: null, expires_at: null }]
    await expect(attachmentLink('M1', 'a', fetch)).rejects.toBeInstanceOf(NoLink)
  })
})
