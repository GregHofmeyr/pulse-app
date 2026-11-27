import { afterEach, describe, expect, it, vi } from 'vitest'
import { withDeadline } from './frames'

describe('withDeadline', () => {
  afterEach(() => vi.useRealTimers())
  it('rejects with the message when nothing arrives in time (a stuck test still logs)', async () => {
    vi.useFakeTimers()
    const p = withDeadline(new Promise(() => {}), 1000, 'no frame 1 s after track')
    vi.advanceTimersByTime(1001)
    await expect(p).rejects.toThrow('no frame 1 s after track')
  })
  it('passes the value through when it arrives first', async () => {
    await expect(withDeadline(Promise.resolve(7), 1000, 'late')).resolves.toBe(7)
  })
})
