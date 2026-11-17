// The image editor's geometry (#290).
import { describe, expect, it } from 'vitest'
import { dragCorner, editedName, fromTurned, fullCrop, growCrop, toTurned, turn, turnCrop, turnedSize, turnMatrix, unchanged, wholeCrop, type Quarter } from './imageEdit'

const apply = (m: number[], p: { x: number; y: number }) => ({ x: m[0] * p.x + m[2] * p.y + m[4], y: m[1] * p.x + m[3] * p.y + m[5] })

describe('quarter turns', () => {
  it('wrap both ways and swap the sides on odd turns', () => {
    expect(turn(3, 1)).toBe(0)
    expect(turn(0, -1)).toBe(3)
    expect(turnedSize(40, 30, 1)).toEqual({ w: 30, h: 40 })
    expect(turnedSize(40, 30, 2)).toEqual({ w: 40, h: 30 })
  })

  it('map points onto the turned picture and back, as the canvas transform does', () => {
    const W = 40
    const H = 30
    for (const q of [0, 1, 2, 3] as Quarter[]) {
      const size = turnedSize(W, H, q)
      for (const p of [{ x: 0, y: 0 }, { x: W, y: 0 }, { x: 7, y: 21 }, { x: W, y: H }]) {
        const t = toTurned(p, W, H, q)
        expect(t.x).toBeGreaterThanOrEqual(0)
        expect(t.x).toBeLessThanOrEqual(size.w)
        expect(t.y).toBeLessThanOrEqual(size.h)
        expect(fromTurned(t, W, H, q)).toEqual(p)
        expect(apply(turnMatrix(W, H, q), p)).toEqual(t)
      }
    }
    // a clockwise turn puts the top-left corner at the top right
    expect(toTurned({ x: 0, y: 0 }, W, H, 1)).toEqual({ x: H, y: 0 })
  })

  it('carry the crop with the picture, and four turns bring it home', () => {
    const c = { x: 5, y: 2, w: 10, h: 6 }
    const once = turnCrop(c, 40, 30, 1)
    expect(once).toEqual({ x: 30 - 8, y: 5, w: 6, h: 10 })
    expect(turnCrop(once, 30, 40, -1)).toEqual(c)
    let r = c
    let size = { w: 40, h: 30 }
    for (let i = 0; i < 4; i++) {
      r = turnCrop(r, size.w, size.h, 1)
      size = { w: size.h, h: size.w }
    }
    expect(r).toEqual(c)
  })
})

describe('the crop', () => {
  it('starts as the whole picture, or the largest centred box of a ratio', () => {
    expect(fullCrop(400, 300, null)).toEqual({ x: 0, y: 0, w: 400, h: 300 })
    expect(fullCrop(400, 300, 1)).toEqual({ x: 50, y: 0, w: 300, h: 300 })
    const wide = fullCrop(400, 300, 16 / 9)
    expect(wide.w).toBe(400)
    expect(wide.h).toBe(225)
    expect(wide.y).toBe(37)
  })

  it('follows a dragged corner, keeping the opposite one and staying on the picture', () => {
    const c = { x: 100, y: 100, w: 100, h: 100 }
    expect(dragCorner(c, 400, 300, 'se', { x: 250, y: 260 }, null)).toEqual({ x: 100, y: 100, w: 150, h: 160 })
    expect(dragCorner(c, 400, 300, 'nw', { x: 50, y: 20 }, null)).toEqual({ x: 50, y: 20, w: 150, h: 180 })
    // past the edge: stops at it
    expect(dragCorner(c, 400, 300, 'se', { x: 900, y: 900 }, null)).toEqual({ x: 100, y: 100, w: 300, h: 200 })
    // past the opposite corner: the smallest box, not a flipped one
    const tiny = dragCorner(c, 400, 300, 'se', { x: 0, y: 0 }, null)
    expect(tiny).toEqual({ x: 100, y: 100, w: 16, h: 16 })
  })

  it('keeps a fixed ratio while a corner is dragged', () => {
    const c = { x: 0, y: 0, w: 160, h: 90 }
    const r = dragCorner(c, 400, 300, 'se', { x: 320, y: 10 }, 16 / 9)
    expect(r.w / r.h).toBeCloseTo(16 / 9)
    expect(r.w).toBe(320)
    // capped by the picture's height
    const big = dragCorner(c, 400, 200, 'se', { x: 999, y: 999 }, 16 / 9)
    expect(big.h).toBeLessThanOrEqual(200)
    expect(big.w).toBeLessThanOrEqual(400)
    expect(big.w / big.h).toBeCloseTo(16 / 9)
  })

  it('grows and shrinks about its centre with the keys, keeping its shape', () => {
    const c = { x: 100, y: 100, w: 100, h: 50 }
    const g = growCrop(c, 400, 300, 1.1)
    expect(g.w).toBeCloseTo(110)
    expect(g.h).toBeCloseTo(55)
    expect(g.x + g.w / 2).toBeCloseTo(150)
    const s = growCrop(c, 400, 300, 1 / 1.1)
    expect(s.w).toBeLessThan(100)
  })

  it('is cut in whole pixels on the picture', () => {
    expect(wholeCrop({ x: 10.4, y: -1, w: 50.6, h: 400 }, 100, 300)).toEqual({ x: 10, y: 0, w: 51, h: 300 })
  })
})

describe('saving', () => {
  it('knows when nothing changed', () => {
    const all = { x: 0, y: 0, w: 40, h: 30 }
    expect(unchanged(all, 40, 30, 0, [])).toBe(true)
    expect(unchanged(all, 40, 30, 1, [])).toBe(false)
    expect(unchanged({ ...all, w: 39 }, 40, 30, 0, [])).toBe(false)
    expect(unchanged(all, 40, 30, 0, [{ color: 'x', width: 4, points: [{ x: 1, y: 1 }] }])).toBe(false)
  })

  it('names the edit as a PNG of the same name', () => {
    expect(editedName('shot.webp')).toBe('shot.png')
    expect(editedName('my.holiday.webp')).toBe('my.holiday.png')
    expect(editedName('noext')).toBe('noext.png')
    expect(editedName('')).toBe('image.png')
  })
})
