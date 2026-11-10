import { describe, expect, it } from 'vitest'
import { COLUMNS, fitColumns, fitStrip, keyWidth, MAIN_MIN, RIGHT_DEFAULT, RIGHT_MIN, STRIP, type FitInput } from './columns'

// #63: the chat strip beside a stream in full view
describe("the stream's chat strip (#63)", () => {
  it('is narrower than the stage chat by default, with the same least width', () => {
    expect(STRIP.def).toBeLessThan(RIGHT_DEFAULT.chat)
    expect(STRIP.min).toBe(RIGHT_MIN.chat)
    expect(STRIP.max).toBe(COLUMNS.right.max)
  })

  it('not laid out yet: only its limits apply', () => {
    expect(fitStrip(null, 0)).toEqual({ width: STRIP.def, max: STRIP.max })
    expect(fitStrip(9999, 0)).toEqual({ width: STRIP.max, max: STRIP.max })
    expect(fitStrip(10, 0)).toEqual({ width: STRIP.min, max: STRIP.max })
  })

  it("leaves the stream at least the main area's least width, never going under its own", () => {
    expect(fitStrip(400, 1200)).toEqual({ width: 400, max: STRIP.max })
    expect(fitStrip(450, 700)).toEqual({ width: 700 - MAIN_MIN, max: 700 - MAIN_MIN })
    // too narrow for both: the strip keeps its least width
    expect(fitStrip(null, 500)).toEqual({ width: STRIP.min, max: STRIP.min })
  })
})

// the minimum window (900px) less the body's 8px side padding
const MIN_BODY = 900 - 16
const fit = (over: Partial<FitInput>) => fitColumns({ saved: { left: null, right: null }, body: 1400, left: true, right: true, rightDefault: RIGHT_DEFAULT.members, ...over })

describe("the voice chat's own minimum (#230)", () => {
  it('the chat keeps its composer usable at 280px; the other right columns keep 180px', () => {
    expect(RIGHT_MIN).toEqual({ members: 180, group: 180, search: 180, chat: 280 })
    const chat = { rightDefault: RIGHT_DEFAULT.chat, rightMin: RIGHT_MIN.chat }
    expect(fit({ ...chat, saved: { left: null, right: 10 } }).right).toBe(280)
    // a shrinking window narrows it to its own minimum, not the members list's
    expect(fit({ ...chat, saved: { left: null, right: 480 }, body: 860 }).right).toBe(280)
    expect(fit({ ...chat, body: 700 }).maxRight).toBe(280)
    expect(fit({ saved: { left: null, right: 10 } }).right).toBe(180)
  })

  it('the smallest window still fits the channel list, the chat at its minimum and the main area', () => {
    expect(COLUMNS.left.def + RIGHT_MIN.chat + MAIN_MIN + 16).toBeLessThanOrEqual(MIN_BODY)
  })
})

describe('column widths (#160)', () => {
  it('unsaved columns are their defaults', () => {
    expect(fit({})).toMatchObject({ left: COLUMNS.left.def, right: RIGHT_DEFAULT.members })
    expect(fit({ rightDefault: RIGHT_DEFAULT.search }).right).toBe(340)
  })

  it('unsaved columns are never narrowed: the smallest window lays out as it did before resizing existed', () => {
    const f = fit({ body: MIN_BODY, rightDefault: RIGHT_DEFAULT.search })
    expect([f.left, f.right]).toEqual([COLUMNS.left.def, RIGHT_DEFAULT.search])
    // with the members list the main area still gets its minimum
    const m = fit({ body: MIN_BODY })
    expect(MIN_BODY - m.left - m.right - 16).toBeGreaterThanOrEqual(MAIN_MIN)
  })

  it('a saved column gives way before a default one does', () => {
    const f = fit({ saved: { left: 400, right: null }, body: MIN_BODY, rightDefault: RIGHT_DEFAULT.search })
    expect(f.right).toBe(RIGHT_DEFAULT.search)
    expect(f.left).toBe(MIN_BODY - 16 - MAIN_MIN - RIGHT_DEFAULT.search)
  })

  it('saved widths stay inside their min and max', () => {
    expect(fit({ saved: { left: 10, right: 9999 } })).toMatchObject({ left: COLUMNS.left.min, right: COLUMNS.right.max })
  })

  it('a shrinking window narrows the widened columns first, never below their minimum, and keeps what was saved', () => {
    const saved = { left: 400, right: 450 }
    const wide = fit({ saved, body: 1600 })
    expect([wide.left, wide.right]).toEqual([400, 450])
    const f = fit({ saved, body: 1000 })
    expect(1000 - f.left - f.right - 16).toBe(MAIN_MIN)
    // both still at or above their defaults, the right one (widened most) given up first
    expect(f.right).toBeGreaterThanOrEqual(RIGHT_DEFAULT.members)
    expect(f.left).toBe(400)
    const tiny = fit({ saved, body: 600 })
    expect([tiny.left, tiny.right]).toEqual([COLUMNS.left.min, COLUMNS.right.min])
    expect(saved).toEqual({ left: 400, right: 450 })
  })

  it('a hidden column takes no room and leaves the other more', () => {
    const f = fit({ saved: { left: 420, right: null }, body: 900, right: false })
    expect(f.left).toBe(420)
    expect(f.maxLeft).toBe(Math.min(COLUMNS.left.max, 900 - 8 - MAIN_MIN))
  })

  it('the most a column may be dragged to keeps the main area at its minimum', () => {
    const f = fit({ body: 900 })
    expect(f.maxLeft).toBe(900 - 16 - RIGHT_DEFAULT.members - MAIN_MIN)
    expect(f.maxRight).toBe(900 - 16 - COLUMNS.left.def - MAIN_MIN)
    // never below the column's own minimum
    expect(fit({ body: 600 }).maxLeft).toBe(COLUMNS.left.min)
  })

  it('an unknown body width (not laid out yet) clamps only to the min and max', () => {
    expect(fit({ saved: { left: 420, right: 480 }, body: 0 })).toMatchObject({ left: 420, right: 480, maxLeft: 420, maxRight: 480 })
  })

  it('arrow keys step the width (the left column grows rightwards, the right one leftwards), Home and End go to the ends', () => {
    expect(keyWidth('left', 264, 'ArrowRight', false, 200, 420)).toBe(280)
    expect(keyWidth('left', 264, 'ArrowLeft', true, 200, 420)).toBe(200)
    expect(keyWidth('right', 240, 'ArrowLeft', false, 180, 480)).toBe(256)
    expect(keyWidth('right', 240, 'ArrowRight', false, 180, 480)).toBe(224)
    expect(keyWidth('right', 470, 'ArrowLeft', false, 180, 480)).toBe(480)
    expect(keyWidth('left', 300, 'Home', false, 200, 420)).toBe(200)
    expect(keyWidth('left', 300, 'End', false, 200, 420)).toBe(420)
    expect(keyWidth('left', 300, 'Enter', false, 200, 420)).toBeNull()
  })
})
