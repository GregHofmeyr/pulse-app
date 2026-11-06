import { describe, expect, it } from 'vitest'
import { flipInverse, stageLayout, tileScale, type Rect } from './stagelayout'

const AR = 16 / 9
const lay = (count: number, width: number, height: number) => stageLayout({ count, width, height })
/** The rows, top to bottom, as tile counts. */
const rows = (rects: Rect[]) => {
  const ys = [...new Set(rects.map((r) => Math.round(r.y)))].sort((a, b) => a - b)
  return ys.map((y) => rects.filter((r) => Math.round(r.y) === y).length)
}
const centreX = (rects: Rect[]) => (Math.min(...rects.map((r) => r.x)) + Math.max(...rects.map((r) => r.x + r.w))) / 2
const overlap = (a: Rect, b: Rect) => a.x < b.x + b.w - 0.01 && b.x < a.x + a.w - 0.01 && a.y < b.y + b.h - 0.01 && b.y < a.y + a.h - 0.01

describe('stageLayout', () => {
  it('nothing to lay out is no tiles', () => {
    expect(lay(0, 1200, 700).rects).toEqual([])
  })

  it('one person nearly fills the stage, centred', () => {
    const { rects } = lay(1, 1200, 700)
    expect(rects).toHaveLength(1)
    const [r] = rects
    // fills the narrower way (here the width, inside the padding)
    expect(r.w).toBeGreaterThan(1200 * 0.9)
    expect(r.w / r.h).toBeCloseTo(AR)
    expect(r.x + r.w / 2).toBeCloseTo(600)
    expect(r.y + r.h / 2).toBeCloseTo(350)
  })

  it('two sit side by side on a wide stage', () => {
    const { rects } = lay(2, 1400, 500)
    expect(rows(rects)).toEqual([2])
  })

  it('two stack on a tall stage', () => {
    const { rects } = lay(2, 600, 900)
    expect(rows(rects)).toEqual([1, 1])
    expect(rects[0].x).toBeCloseTo(rects[1].x)
  })

  it('three make a pyramid: two over one, the one centred', () => {
    const { rects } = lay(3, 1200, 700)
    expect(rows(rects)).toEqual([2, 1])
    expect(rects[2].x + rects[2].w / 2).toBeCloseTo(600)
  })

  it('four make a 2x2', () => {
    expect(rows(lay(4, 1200, 700).rects)).toEqual([2, 2])
  })

  it('five are 3 + 2 and six 3 x 2, the short row centred', () => {
    const five = lay(5, 1400, 700).rects
    expect(rows(five)).toEqual([3, 2])
    const last = five.slice(3)
    expect(centreX(last)).toBeCloseTo(700)
    expect(rows(lay(6, 1400, 700).rects)).toEqual([3, 3])
  })

  it('the whole arrangement is centred both ways', () => {
    const { rects } = lay(4, 1600, 700)
    expect(centreX(rects)).toBeCloseTo(800)
    const top = Math.min(...rects.map((r) => r.y))
    const bottom = Math.max(...rects.map((r) => r.y + r.h))
    expect((top + bottom) / 2).toBeCloseTo(350)
  })

  it('picks the rows that make the biggest tiles', () => {
    for (const [n, w, h] of [[3, 1200, 700], [5, 900, 900], [7, 1600, 800], [8, 700, 1200], [3, 2400, 400]] as const) {
      const got = lay(n, w, h).rects[0].w
      // every other row count, by the same rule: no bigger tile is possible
      for (let r = 1; r <= n; r++) {
        const c = Math.ceil(n / r)
        const rr = Math.ceil(n / c)
        const alt = Math.min((w - 40 - (c - 1) * 14) / c, ((h - 40 - (rr - 1) * 14) / rr) * AR)
        expect(got).toBeGreaterThanOrEqual(Math.min(alt, 1440) - 0.01)
      }
    }
  })

  it('tiles keep the aspect, the same size, inside the stage and apart', () => {
    for (const [w, h] of [[1200, 700], [700, 1100], [2000, 500], [500, 400]]) {
      for (let n = 1; n <= 12; n++) {
        const { rects } = lay(n, w, h)
        expect(rects).toHaveLength(n)
        for (const r of rects) {
          expect(r.w / r.h).toBeCloseTo(AR)
          expect(r.w).toBeCloseTo(rects[0].w)
          expect(r.x).toBeGreaterThanOrEqual(0)
          expect(r.x + r.w).toBeLessThanOrEqual(w + 0.01)
          if (r.w > 192) expect(r.y + r.h).toBeLessThanOrEqual(h + 0.01)
        }
        for (let i = 0; i < n; i++) for (let j = i + 1; j < n; j++) expect(overlap(rects[i], rects[j])).toBe(false)
      }
    }
  })

  it('a tile never grows past the cap, however big the stage', () => {
    const { rects } = lay(1, 4000, 2400)
    expect(rects[0].w).toBe(1440)
    expect(rects[0].x + rects[0].w / 2).toBeCloseTo(2000)
  })

  it('a crowded small stage keeps tiles at the smallest size and scrolls', () => {
    const { rects, height } = lay(20, 700, 400)
    expect(rects[0].w).toBe(192)
    // three to a row fit in 700px
    expect(rows(rects)[0]).toBe(3)
    expect(height).toBeGreaterThan(400)
    // the content starts at the top (it scrolls from there)
    expect(Math.min(...rects.map((r) => r.y))).toBe(20)
  })

  it('a stage not yet measured gives finite rects', () => {
    for (const r of lay(3, 0, 0).rects) for (const v of [r.x, r.y, r.w, r.h]) expect(Number.isFinite(v)).toBe(true)
  })

  it('takes its own aspect, gap and padding', () => {
    const { rects } = stageLayout({ count: 2, width: 1000, height: 1000, aspect: 1, gap: 0, pad: 0 })
    expect(rects[0].w).toBeCloseTo(rects[0].h)
    expect(rects[0].w + rects[1].w).toBeLessThanOrEqual(1000.01)
  })
})

describe('tileScale', () => {
  it('a bigger tile has a bigger avatar and name', () => {
    const small = tileScale(140)
    const big = tileScale(400)
    expect(big.avatar).toBeGreaterThan(small.avatar)
    expect(big.font).toBeGreaterThan(small.font)
    expect(big.badge).toBeGreaterThanOrEqual(small.badge)
  })

  it('stays within limits', () => {
    expect(tileScale(40)).toEqual(tileScale(60))
    expect(tileScale(5000)).toEqual(tileScale(2000))
    expect(tileScale(5000).avatar).toBeLessThanOrEqual(160)
    expect(tileScale(40).avatar).toBeGreaterThanOrEqual(48)
    expect(tileScale(40).font).toBe(14)
  })

  it('badge icons use the icon sizes (14 or 18)', () => {
    for (const h of [60, 150, 300, 800]) expect([14, 18]).toContain(tileScale(h).icon)
  })

  it('whole pixels', () => {
    const s = tileScale(333)
    for (const v of Object.values(s)) expect(Number.isInteger(v)).toBe(true)
  })
})

describe('flipInverse', () => {
  it('moves and scales a tile back to where it was', () => {
    const inv = flipInverse({ x: 100, y: 50, w: 200, h: 100 }, { x: 300, y: 150, w: 400, h: 200 })
    expect(inv).toEqual({ dx: -200, dy: -100, sx: 0.5, sy: 0.5 })
  })

  it('nothing to do when it has not moved', () => {
    expect(flipInverse({ x: 1, y: 2, w: 3, h: 4 }, { x: 1.2, y: 2, w: 3, h: 4 })).toBeNull()
  })

  it('nothing to do from or to an empty box', () => {
    expect(flipInverse({ x: 0, y: 0, w: 0, h: 0 }, { x: 1, y: 2, w: 3, h: 4 })).toBeNull()
    expect(flipInverse({ x: 1, y: 2, w: 3, h: 4 }, { x: 0, y: 0, w: 0, h: 0 })).toBeNull()
  })
})
