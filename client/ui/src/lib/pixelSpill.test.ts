import { describe, expect, it } from 'vitest'
import { pixelSpill } from './pixelSpill'

describe('pixelSpill', () => {
  it('same seed same pixels', () => {
    expect(pixelSpill('E1', 58, 8)).toEqual(pixelSpill('E1', 58, 8))
    expect(pixelSpill('E1', 58, 8).length).toBeGreaterThan(0)
  })

  it('different seeds differ', () => {
    expect(pixelSpill('E1', 58, 8)).not.toEqual(pixelSpill('E2', 58, 8))
  })

  it('pixels stay inside the grid and use three shades', () => {
    const px = pixelSpill('0193a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b', 58, 8)
    for (const p of px) {
      expect(Number.isInteger(p.x) && p.x >= 0 && p.x < 58).toBe(true)
      expect(Number.isInteger(p.y) && p.y >= 0 && p.y < 8).toBe(true)
    }
    expect(new Set(px.map((p) => p.shade))).toEqual(new Set([0, 1, 2]))
    // one pixel per cell
    expect(new Set(px.map((p) => `${p.x},${p.y}`)).size).toBe(px.length)
  })

  it('is solid at the edge and thins out away from it', () => {
    const px = pixelSpill('E1', 58, 8)
    const inCol = (x: number) => px.filter((p) => p.x === x).length
    expect(inCol(0)).toBe(8)
    const near = px.filter((p) => p.x < 19).length
    const far = px.filter((p) => p.x >= 39).length
    expect(far).toBeLessThan(near / 2)
  })
})
