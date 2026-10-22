import { describe, expect, it } from 'vitest'
import { Ring, strain } from './sampler'

describe('Ring', () => {
  it('ring_never_grows_past_its_capacity', () => {
    const r = new Ring<number>(3)
    for (let i = 0; i < 10; i++) r.push(i)
    expect(r.items()).toEqual([7, 8, 9])
  })
})

describe('strain', () => {
  const s = (t: number, frames: number, encSec: number, reason = 'none') =>
    ({ t, framesEncoded: frames, totalEncodeTime: encSec, framesSent: frames, qualityLimitationReason: reason })
  it('flags low fps, heavy encode and cpu limits (spec 6.2)', () => {
    expect(strain(s(0, 0, 0), s(2000, 120, 0.2), 60)).toEqual({ lowFps: false, encodeHeavy: false, cpuLimited: false })
    expect(strain(s(0, 0, 0), s(2000, 80, 0.2), 60).lowFps).toBe(true) // 40 fps < 85% of 60
    expect(strain(s(0, 0, 0), s(2000, 120, 1.5), 60).encodeHeavy).toBe(true) // 12.5 ms/frame > 70% of 16.7
    expect(strain(s(0, 0, 0), s(2000, 120, 0.2, 'cpu'), 60).cpuLimited).toBe(true)
  })
})
