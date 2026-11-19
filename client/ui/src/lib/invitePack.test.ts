// @vitest-environment jsdom
// The invite card pack (#511), the pure part: who gets it sealed, how a drag across the top tears it (either way,
// springing back if let go early, ripping at TEAR_AT), and remembering on this device that it was opened.
import { afterEach, describe, expect, it, vi } from 'vitest'
import { BAND_PX, NO_TEAR, TEAR_AT, markOpened, press, progressOf, startStage, stretch, wasOpened } from './invitePack'

const base = { mine: false, valid: true, reducedMotion: false, opened: false }

describe('invite pack', () => {
  afterEach(() => {
    localStorage.clear()
    vi.restoreAllMocks()
  })

  it('arrives sealed only for someone it is for, still valid, never opened, with motion', () => {
    expect(startStage(base)).toBe('sealed')
    // its sender sees their card open
    expect(startStage({ ...base, mine: true })).toBe('open')
    // dead (or already in): the card, no pack
    expect(startStage({ ...base, valid: false })).toBe('open')
    // opened on this device before
    expect(startStage({ ...base, opened: true })).toBe('open')
    // less motion: just open
    expect(startStage({ ...base, reducedMotion: true })).toBe('open')
  })

  it('a tear starts only on the top band, and stretches either way', () => {
    expect(press(0.5, BAND_PX + 1)).toBeNull()
    let t = press(0.5, 40)!
    expect(t).toEqual({ dragging: true, lo: 0.5, hi: 0.5, cursor: 0.5 })
    expect(progressOf(t)).toBe(0)
    t = stretch(t, 0.2).tear
    expect(progressOf(t)).toBeCloseTo(0.3)
    t = stretch(t, 0.7).tear
    expect([t.lo, t.hi, t.cursor]).toEqual([0.2, 0.7, 0.7])
    // going back doesn't shrink it
    t = stretch(t, 0.4).tear
    expect(progressOf(t)).toBeCloseTo(0.5)
    // off the edge counts as the edge
    t = stretch(t, 1.6).tear
    expect(t.hi).toBe(1)
  })

  it(`rips at ${TEAR_AT * 100}% of the width; let go early and it springs back`, () => {
    let t = press(0.05, 10)!
    let r = stretch(t, 0.6)
    expect(r.rip).toBe(false)
    r = stretch(r.tear, 0.05 + TEAR_AT)
    expect(r.rip).toBe(true)
    // released before that: nothing torn
    t = stretch(press(0.9, 10)!, 0.5).tear
    expect(progressOf(NO_TEAR)).toBe(0)
    expect(stretch(NO_TEAR, 0.1)).toEqual({ tear: NO_TEAR, rip: false })
    expect(progressOf(t)).toBeGreaterThan(0)
  })

  it('remembers on this device that it was opened, per invite', () => {
    expect(wasOpened('abcDEF2345')).toBe(false)
    markOpened('abcDEF2345')
    expect(wasOpened('abcDEF2345')).toBe(true)
    expect(wasOpened('zzzDEF2345')).toBe(false)
  })

  it('storage that refuses: never opened before, and opening still works', () => {
    vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => {
      throw new Error('denied')
    })
    vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
      throw new Error('denied')
    })
    expect(wasOpened('abcDEF2345')).toBe(false)
    expect(() => markOpened('abcDEF2345')).not.toThrow()
  })
})
