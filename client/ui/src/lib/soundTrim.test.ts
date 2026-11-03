import { describe, expect, it } from 'vitest'
import { MAX_SPAN_MS, STEP_MS, initialSpan, moveEnd, moveStart, msAt, spanLabel } from './soundTrim'

describe('soundTrim', () => {
  it('starts at the beginning, at most 6 s long', () => {
    expect(initialSpan(2500)).toEqual({ start: 0, end: 2500 })
    expect(initialSpan(20000)).toEqual({ start: 0, end: MAX_SPAN_MS })
  })
  it('moving the end past 6 s drags the start along', () => {
    expect(moveEnd({ start: 1000, end: 6000 }, 9000, 20000)).toEqual({ start: 3000, end: 9000 })
  })
  it('moving the start back past 6 s drags the end along', () => {
    expect(moveStart({ start: 6000, end: 12000 }, 2000, 20000)).toEqual({ start: 2000, end: 8000 })
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
