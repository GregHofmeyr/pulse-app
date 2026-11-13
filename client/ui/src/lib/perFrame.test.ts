import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { perFrame } from './perFrame'

// A native colour picker fires `input` many times a frame while you drag (#156 follow-up: Settings flashed on Linux).
describe('perFrame', () => {
  let frames: FrameRequestCallback[] = []
  beforeEach(() => {
    frames = []
    vi.stubGlobal('requestAnimationFrame', (cb: FrameRequestCallback) => frames.push(cb))
    vi.stubGlobal('cancelAnimationFrame', () => {})
  })
  afterEach(() => vi.unstubAllGlobals())
  const frame = () => frames.splice(0).forEach((f) => f(0))

  it('a burst of values in one frame is applied once, with the last', () => {
    const apply = vi.fn()
    const p = perFrame(apply)
    for (let i = 0; i < 20; i++) p.push(`#0000${i.toString(16).padStart(2, '0')}`)
    expect(apply).not.toHaveBeenCalled()
    expect(frames).toHaveLength(1)
    frame()
    expect(apply.mock.calls).toEqual([['#000013']])
    p.push('#ffffff')
    frame()
    expect(apply).toHaveBeenCalledTimes(2)
  })

  it('flush applies a pending value at once (the picker closing), and only once', () => {
    const apply = vi.fn()
    const p = perFrame(apply)
    p.push('#123456')
    p.flush()
    expect(apply.mock.calls).toEqual([['#123456']])
    frame()
    p.flush()
    expect(apply).toHaveBeenCalledTimes(1)
  })

  it('cancel drops a pending value', () => {
    const apply = vi.fn()
    const p = perFrame(apply)
    p.push('#123456')
    p.cancel()
    frame()
    expect(apply).not.toHaveBeenCalled()
  })
})
