// Weighted wheels and palettes (#259): each slice's share of the turn, the pointer reading it, the private draw being
// fair to the weights, and the palettes being theme tokens.
import { describe, expect, it } from 'vitest'
import { cleanWeights, drawSpin, sliceSpans, weightsFor, winnerAt } from './wheel'
import { PALETTES, paletteOf, sliceStyle } from './wheelColours'

/** A seeded generator (mulberry32), so the fairness check is the same every run. */
function seeded(seed: number): () => number {
  let a = seed >>> 0
  return () => {
    a = (a + 0x6d2b79f5) >>> 0
    let t = a
    t = Math.imul(t ^ (t >>> 15), t | 1)
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61)
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296
  }
}

describe('weighted slices', () => {
  it('cover their share of the turn, in order from 12 o\'clock', () => {
    expect(sliceSpans(2, [1, 3])).toEqual([{ from: 0, seg: 90 }, { from: 90, seg: 270 }])
    expect(sliceSpans(4)).toEqual([0, 90, 180, 270].map((from) => ({ from, seg: 90 })))
    // weights that don't fit the options are ignored
    expect(sliceSpans(3, [1, 2])).toEqual(sliceSpans(3))
    expect(weightsFor(3, null)).toEqual([1, 1, 1])
  })

  it('are read by the pointer as the server reads them (wheel_winner_at_weighted)', () => {
    expect(winnerAt(360 - 45, 2, [1, 3])).toBe(0)
    expect(winnerAt(360 - 91, 2, [1, 3])).toBe(1)
    expect(winnerAt(1, 2, [1, 3])).toBe(1)
    expect(winnerAt(0, 2, [1, 3])).toBe(0)
    // all alike reads as the plain wheel
    for (let a = 0.5; a < 360; a += 7) expect(winnerAt(a, 5, [2, 2, 2, 2, 2])).toBe(winnerAt(a, 5))
  })

  it('all alike is no weights, as the server stores it', () => {
    expect(cleanWeights([1, 1, 1])).toBeUndefined()
    expect(cleanWeights([3, 3])).toBeUndefined()
    expect(cleanWeights([1, 4])).toEqual([1, 4])
  })
})

describe("a private spin's draw", () => {
  it('wins in proportion to the weights and lands inside its slice', () => {
    const random = seeded(259)
    const weights = [1, 2, 3, 4]
    const wins = [0, 0, 0, 0]
    const runs = 40_000
    for (let i = 0; i < runs; i++) {
      const { winner, angle } = drawSpin(4, random, weights)
      expect(winnerAt(angle, 4, weights)).toBe(winner)
      wins[winner]++
    }
    weights.forEach((w, i) => expect(Math.abs(wins[i] / runs - w / 10)).toBeLessThan(0.015))
  })

  it('maps the random number onto the weights exactly', () => {
    // [1, 3]: below a quarter is the first option, from a quarter on the second
    expect(drawSpin(2, () => 0.24, [1, 3]).winner).toBe(0)
    expect(drawSpin(2, () => 0.25, [1, 3]).winner).toBe(1)
    expect(drawSpin(2, () => 0.99, [1, 3]).winner).toBe(1)
    // no weights: as before
    expect(drawSpin(4, () => 0.5).winner).toBe(2)
  })
})

describe('palettes', () => {
  it('are the neon spectrum by default and theme tokens otherwise', () => {
    expect(PALETTES[0].id).toBe('neon')
    expect(paletteOf(null)).toBe('neon')
    expect(paletteOf('plaid')).toBe('neon')
    expect(sliceStyle(0, 4, null)).toEqual({ fill: 'hsl(0, 100%, 58%)', ink: expect.any(String), edged: false })
    for (const p of PALETTES.slice(1)) {
      for (let i = 0; i < 7; i++) {
        const s = sliceStyle(i, 7, p.id)
        expect(s.fill).toMatch(/^var\(--[a-z0-9-]+\)$/)
        expect(s.ink).toBe('var(--on-video)')
        expect(s.edged).toBe(true)
      }
    }
  })

  it('never put the same colour on the last slice and the first, which touch', () => {
    for (const p of PALETTES.slice(1)) {
      for (let n = 2; n <= 13; n++) expect(sliceStyle(n - 1, n, p.id).fill, `${p.id} n=${n}`).not.toBe(sliceStyle(0, n, p.id).fill)
    }
  })

  it("name the server's palettes", () => {
    // protocol/src/rest.rs wheel_palettes::ALL
    expect(PALETTES.map((p) => p.id)).toEqual(['neon', 'gems', 'signal', 'meadow', 'accent'])
  })
})
