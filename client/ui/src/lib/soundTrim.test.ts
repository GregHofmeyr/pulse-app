import { describe, expect, it } from 'vitest'
import { MAX_SPAN_MS, STEP_MS, initialSpan, moveEnd, moveSpan, moveStart, msAt, playFrom, spanLabel } from './soundTrim'

describe('soundTrim', () => {
  it('starts at the beginning, at most 6 s long', () => {
    expect(initialSpan(2500)).toEqual({ start: 0, end: 2500 })
    expect(initialSpan(20000)).toEqual({ start: 0, end: MAX_SPAN_MS })
  })
  it('the end handle moves only the end and stops at 6 s', () => {
    expect(moveEnd({ start: 1000, end: 6000 }, 9000, 20000)).toEqual({ start: 1000, end: 7000 })
    expect(moveEnd({ start: 1000, end: 6000 }, 4000, 20000)).toEqual({ start: 1000, end: 4000 })
  })
  it('the start handle moves only the start and stops at 6 s', () => {
    expect(moveStart({ start: 6000, end: 12000 }, 2000, 20000)).toEqual({ start: 6000, end: 12000 })
    expect(moveStart({ start: 7000, end: 12000 }, 2000, 20000)).toEqual({ start: 6000, end: 12000 })
    expect(moveStart({ start: 7000, end: 12000 }, 9000, 20000)).toEqual({ start: 9000, end: 12000 })
  })
  it('dragging the selection moves it whole, inside the clip', () => {
    expect(moveSpan({ start: 1000, end: 4000 }, 2500, 20000)).toEqual({ start: 3500, end: 6500 })
    expect(moveSpan({ start: 1000, end: 4000 }, -5000, 20000)).toEqual({ start: 0, end: 3000 })
    expect(moveSpan({ start: 1000, end: 4000 }, 99999, 20000)).toEqual({ start: 17000, end: 20000 })
    expect(moveSpan({ start: 0, end: 5000 }, 300, 5000)).toEqual({ start: 0, end: 5000 })
  })
  it('playback starts at a paused playhead inside the span, else at the start', () => {
    const span = { start: 1000, end: 4000 }
    expect(playFrom(span, null)).toBe(1000)
    expect(playFrom(span, 2500)).toBe(2500)
    expect(playFrom(span, 4000)).toBe(1000)
    expect(playFrom(span, 500)).toBe(1000)
  })
  it('keeps both handles inside the clip and apart by a step', () => {
    expect(moveEnd({ start: 0, end: 3000 }, 99999, 5000)).toEqual({ start: 0, end: 5000 })
    expect(moveStart({ start: 1000, end: 3000 }, -50, 5000)).toEqual({ start: 0, end: 3000 })
    expect(moveStart({ start: 0, end: 3000 }, 3000, 5000)).toEqual({ start: 3000 - STEP_MS, end: 3000 })
    expect(moveEnd({ start: 1000, end: 3000 }, 0, 5000)).toEqual({ start: 1000, end: 1000 + STEP_MS })
  })
  it('maps a position along the waveform to a rounded millisecond', () => {
    expect(msAt(0.5, 3001)).toBe(1501)
    expect(msAt(-1, 3000)).toBe(0)
    expect(msAt(2, 3000)).toBe(3000)
  })
  it('labels the span in seconds', () => {
    expect(spanLabel({ start: 500, end: 3700 })).toBe('3.2 s')
  })
})
