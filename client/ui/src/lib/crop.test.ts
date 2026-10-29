import { describe, expect, it } from 'vitest'
import { ASPECT, initialCrop, panCrop, roundCrop, zoomCrop, type CropRect } from './crop'

const within = (c: CropRect, w: number, h: number) => c.x >= 0 && c.y >= 0 && c.w >= 1 && c.h >= 1 && c.x + c.w <= w && c.y + c.h <= h
const inside = (c: CropRect, w: number, h: number) => [c.x, c.y, c.w, c.h].every(Number.isInteger) && within(c, w, h)

describe('crop', () => {
  it('initial crop is centred and maximal for both aspects', () => {
    expect(initialCrop(800, 600, ASPECT.avatar)).toEqual({ x: 100, y: 0, w: 600, h: 600 })
    expect(initialCrop(600, 800, ASPECT.avatar)).toEqual({ x: 0, y: 100, w: 600, h: 600 })
    expect(initialCrop(1920, 1080, ASPECT.banner)).toEqual({ x: 0, y: 156, w: 1920, h: 768 })
    expect(initialCrop(960, 1000, ASPECT.banner)).toEqual({ x: 0, y: 308, w: 960, h: 384 })
  })

  it('pan clamps at every edge', () => {
    const c = initialCrop(800, 600, ASPECT.avatar)
    expect(panCrop(c, 800, 600, -999, 0).x).toBe(0)
    expect(panCrop(c, 800, 600, 999, 0).x).toBe(200)
    expect(panCrop(c, 800, 600, 0, -999).y).toBe(0)
    expect(panCrop(c, 800, 600, 0, 999).y).toBe(0)
    const z = zoomCrop(c, 800, 600, 1, 2, { x: 400, y: 300 })
    expect(panCrop(z, 800, 600, 0, 999).y).toBe(600 - z.h)
    expect(panCrop(z, 800, 600, 10, 5)).toMatchObject({ x: z.x + 10, y: z.y + 5, w: z.w, h: z.h })
  })

  it('zoom keeps the aspect and focus and stops at 32 px', () => {
    const c = initialCrop(1000, 1000, ASPECT.avatar)
    const z = zoomCrop(c, 1000, 1000, 1, 2, { x: 250, y: 250 })
    expect(z.w).toBe(500)
    expect(z.h).toBe(500)
    // the focus keeps its relative place in the rectangle
    expect((250 - z.x) / z.w).toBeCloseTo(0.25, 1)
    expect(zoomCrop(c, 1000, 1000, 1, 1000, { x: 500, y: 500 })).toMatchObject({ w: 32, h: 32 })
    const b = initialCrop(1920, 768, ASPECT.banner)
    const zb = zoomCrop(b, 1920, 768, ASPECT.banner, 1000, { x: 900, y: 300 })
    expect(zb.h).toBe(32)
    expect(Math.abs(zb.w / zb.h - ASPECT.banner)).toBeLessThan(0.05)
    // zooming out never passes the image
    expect(zoomCrop(z, 1000, 1000, 1, 0.001, { x: 0, y: 0 })).toEqual({ x: 0, y: 0, w: 1000, h: 1000 })
  })

  it('stays inside the image while moving, and rounds to integers inside it', () => {
    for (const [w, h] of [[801, 603], [333, 1001], [4096, 77], [50, 50]]) {
      for (const a of [ASPECT.avatar, ASPECT.banner]) {
        let c = initialCrop(w, h, a)
        expect(inside(c, w, h)).toBe(true)
        for (const f of [1.37, 0.83, 3.3, 0.5, 7.1]) {
          c = zoomCrop(c, w, h, a, f, { x: w * 0.3, y: h * 0.8 })
          expect(within(c, w, h)).toBe(true)
          expect(inside(roundCrop(c, w, h), w, h)).toBe(true)
          c = panCrop(c, w, h, 12.7, -33.3)
          expect(within(c, w, h)).toBe(true)
          expect(inside(roundCrop(c, w, h), w, h)).toBe(true)
        }
      }
    }
  })

  it('sub-pixel moves add up (nothing is rounded away mid-drag)', () => {
    let c = initialCrop(800, 600, ASPECT.avatar)
    for (let i = 0; i < 10; i++) c = panCrop(c, 800, 600, -0.4, 0)
    expect(roundCrop(c, 800, 600)).toEqual({ x: 96, y: 0, w: 600, h: 600 })
    let z = initialCrop(800, 600, ASPECT.avatar)
    for (let i = 0; i < 20; i++) z = zoomCrop(z, 800, 600, 1, 1.001, { x: 400, y: 300 })
    expect(roundCrop(z, 800, 600).w).toBe(Math.round(600 / 1.001 ** 20))
  })
})
