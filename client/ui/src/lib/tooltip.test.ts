import { describe, expect, it } from 'vitest'
import { placeTip } from './tooltip'

const vp = { width: 1000, height: 800 }

describe('placeTip', () => {
  it('centres above the target', () => {
    const p = placeTip({ left: 480, top: 400, width: 40, height: 30 }, { width: 100, height: 28 }, vp)
    expect(p).toEqual({ left: 450, top: 364, below: false })
  })
  it('flips below when there is no room above', () => {
    const p = placeTip({ left: 480, top: 10, width: 40, height: 30 }, { width: 100, height: 28 }, vp)
    expect(p.below).toBe(true)
    expect(p.top).toBe(48)
  })
  it('stays inside the viewport horizontally', () => {
    expect(placeTip({ left: 0, top: 400, width: 20, height: 20 }, { width: 120, height: 28 }, vp).left).toBe(8)
    expect(placeTip({ left: 990, top: 400, width: 10, height: 20 }, { width: 120, height: 28 }, vp).left).toBe(872)
  })
})
