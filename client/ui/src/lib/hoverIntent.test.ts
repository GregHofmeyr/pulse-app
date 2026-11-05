import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { closeTimer, SUBMENU_CLOSE_DELAY } from './hoverIntent'

describe('closeTimer (#173: submenu hover intent)', () => {
  beforeEach(() => void vi.useFakeTimers())
  afterEach(() => void vi.useRealTimers())

  it('closes after the delay, not at once', () => {
    const close = vi.fn()
    closeTimer().schedule(close)
    vi.advanceTimersByTime(SUBMENU_CLOSE_DELAY - 1)
    expect(close).not.toHaveBeenCalled()
    vi.advanceTimersByTime(1)
    expect(close).toHaveBeenCalledOnce()
  })

  it('arriving back cancels it, and a new schedule restarts the wait', () => {
    const close = vi.fn()
    const t = closeTimer()
    t.schedule(close)
    vi.advanceTimersByTime(150)
    t.cancel()
    vi.advanceTimersByTime(500)
    expect(close).not.toHaveBeenCalled()
    t.schedule(close)
    vi.advanceTimersByTime(150)
    t.schedule(close)
    vi.advanceTimersByTime(150)
    expect(close).not.toHaveBeenCalled()
    vi.advanceTimersByTime(50)
    expect(close).toHaveBeenCalledOnce()
  })

  it('waits between 150 and 250ms (long enough to cross an item, short enough not to feel stuck)', () => {
    expect(SUBMENU_CLOSE_DELAY).toBeGreaterThanOrEqual(150)
    expect(SUBMENU_CLOSE_DELAY).toBeLessThanOrEqual(250)
  })
})
