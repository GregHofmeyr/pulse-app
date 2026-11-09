import { describe, expect, it } from 'vitest'
import {
  addOptions, fitLabel, labelFit, QUIET_SPIN_MS, rotationAt, shuffled, SPIN_MS, SPIN_TURNS,
  splitOptions, winnerAt,
} from './wheel'
import { contrast, LABEL_DARK, LABEL_LIGHT, labelColour, sliceColour, sliceRgb } from './wheelColours'

// the `/wheel` command itself is in the command registry (lib/commands.ts, #262)

describe('editing options', () => {
  it('splits pasted lists by commas and lines', () => {
    expect(splitOptions('a, b\nc,, ')).toEqual(['a', 'b', 'c'])
  })
  it('adds new ones, skips ones already there in any case, cuts long ones', () => {
    expect(addOptions(['Pizza'], 'pizza, sushi, Sushi')).toEqual({ options: ['Pizza', 'sushi'], full: false })
    expect(addOptions([], 'x'.repeat(70)).options[0]).toHaveLength(60)
  })
  it('stops at 100', () => {
    const many = Array.from({ length: 99 }, (_, i) => `o${i}`)
    expect(addOptions(many, 'a, b, c')).toEqual({ options: [...many, 'a'], full: true })
  })
  it('shuffles into a permutation', () => {
    const list = ['a', 'b', 'c', 'd']
    const out = shuffled(list, () => 0)
    expect([...out].sort()).toEqual(list)
    expect(out).not.toEqual(list)
    expect(list).toEqual(['a', 'b', 'c', 'd'])
  })
})

describe('spin geometry (as the server draws it)', () => {
  it('reads the slice under the pointer like wheel_winner_at', () => {
    expect(winnerAt(0, 4)).toBe(0)
    expect(winnerAt(359, 4)).toBe(0)
    expect(winnerAt(269, 4)).toBe(1)
    expect(winnerAt(179, 4)).toBe(2)
    expect(winnerAt(1, 4)).toBe(3)
    expect(winnerAt(5 * 360 + 269, 4)).toBe(1)
    expect(winnerAt(-91, 4)).toBe(1)
    for (let n = 2; n <= 100; n++) {
      const seg = 360 / n
      for (let i = 0; i < n; i++) expect(winnerAt(360 - seg * (i + 0.5), n)).toBe(i)
    }
  })
  it('a spin starts still, turns five times and ends exactly on the angle', () => {
    expect(rotationAt(0, 123.4, false)).toBe(0)
    expect(rotationAt(SPIN_MS, 123.4, false)).toBeCloseTo(SPIN_TURNS * 360 + 123.4)
    expect(rotationAt(SPIN_MS * 2, 123.4, false)).toBeCloseTo(SPIN_TURNS * 360 + 123.4)
    // eased: most of the turning happens early
    expect(rotationAt(SPIN_MS / 2, 100, false)).toBeGreaterThan((SPIN_TURNS * 360 + 100) * 0.9)
    expect(winnerAt(rotationAt(SPIN_MS, 300, false), 7)).toBe(winnerAt(300, 7))
  })
  it('a quiet spin is short and only turns to the landing', () => {
    expect(QUIET_SPIN_MS).toBeLessThanOrEqual(500)
    expect(rotationAt(QUIET_SPIN_MS, 200, true)).toBeCloseTo(200)
  })
  it('labels shrink and cut on crowded wheels', () => {
    expect(labelFit(4).size).toBe(9)
    expect(labelFit(100).size).toBeLessThan(4)
    expect(labelFit(8).chars).toBeGreaterThanOrEqual(12)
    expect(fitLabel('Deep Rock Galactic', 8)).toBe('Deep Ro…')
    expect(fitLabel('Apex', 8)).toBe('Apex')
  })
})

describe('slice colours (the neon spectrum)', () => {
  it('runs evenly through the hues at full saturation', () => {
    expect(sliceColour(0, 4)).toBe('hsl(0, 100%, 58%)')
    expect(sliceColour(1, 4)).toBe('hsl(90, 100%, 58%)')
    expect(sliceColour(2, 3)).toBe('hsl(240, 100%, 58%)')
    expect(sliceColour(1, 7)).toBe('hsl(51.43, 100%, 58%)')
  })
  it('matches CSS hsl() when converted', () => {
    expect(sliceRgb(0, 3)).toEqual([255, 41, 41])
    expect(sliceRgb(1, 3)).toEqual([41, 255, 41])
  })
  it('dark text on the light hues, white on the dark ones', () => {
    expect(labelColour(0, 6)).toBe(LABEL_LIGHT) // red
    expect(labelColour(1, 6)).toBe(LABEL_DARK) // yellow
    expect(labelColour(2, 6)).toBe(LABEL_DARK) // green
    expect(labelColour(3, 6)).toBe(LABEL_DARK) // cyan
    expect(labelColour(4, 6)).toBe(LABEL_LIGHT) // blue
  })
  it('every label reads (3:1 or better) on every slice of every wheel size', () => {
    let worst = Infinity
    for (let n = 2; n <= 100; n++) {
      for (let i = 0; i < n; i++) worst = Math.min(worst, contrast(labelColour(i, n), sliceRgb(i, n)))
    }
    expect(worst).toBeGreaterThanOrEqual(3)
  })
})
