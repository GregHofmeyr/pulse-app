import { afterEach, describe, expect, it, vi } from 'vitest'
import { frameLoop, withDeadline } from './frames'

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

/** A fake <video> whose frame callbacks we fire by hand. */
function fakeVideo() {
  type Cb = (now: number, meta: VideoFrameCallbackMetadata) => void
  let cbs: Cb[] = []
  return {
    requestVideoFrameCallback(cb: Cb) {
      cbs.push(cb)
      return cbs.length
    },
    frame() {
      const now = cbs
      cbs = []
      for (const cb of now) cb(0, {} as VideoFrameCallbackMetadata)
    },
  }
}

describe('frameLoop', () => {
  it('a restarted loop counts each frame once (the old chain stops)', () => {
    const v = fakeVideo()
    let n = 0
    const stop1 = frameLoop(v, () => n++)
    v.frame()
    stop1()
    frameLoop(v, () => n++) // re-Watch
    v.frame()
    v.frame()
    expect(n).toBe(3)
  })
})
