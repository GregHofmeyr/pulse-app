import { describe, expect, it } from 'vitest'
import { addDays, addMonths, clampDay, dayStartIso, mediumDay, monthGrid, parseDay, shortDay, toDay } from './calendar'

describe('calendar', () => {
  it('a day is YYYY-MM-DD in local time, both ways', () => {
    expect(toDay(new Date(2026, 10, 1, 23, 59))).toBe('2026-11-01')
    expect(parseDay('2026-11-01')?.getTime()).toBe(new Date(2026, 10, 1).getTime())
    expect(parseDay('nope')).toBeNull()
    expect(parseDay('2026-02-31')).toBeNull()
  })

  it('the start of a local day as an instant', () => {
    expect(dayStartIso('2026-09-01')).toBe(new Date(2026, 8, 1).toISOString())
  })

  it('days and months step across month and year ends', () => {
    expect(addDays('2026-11-30', 1)).toBe('2026-12-01')
    expect(addDays('2026-01-01', -1)).toBe('2025-12-31')
    expect(addDays('2026-11-01', 7)).toBe('2026-11-08')
    // the 31st into a shorter month lands on its last day
    expect(addMonths('2026-01-31', 1)).toBe('2026-02-28')
    expect(addMonths('2026-01-15', -1)).toBe('2025-12-15')
  })

  it('a month is always 6 weeks of 7, Monday first, so the grid never changes height', () => {
    const nov = monthGrid(2026, 10)
    expect(nov).toHaveLength(6)
    for (const w of nov) expect(w).toHaveLength(7)
    // 1 Nov 2026 is a Sunday: the grid starts on Monday 26 Oct
    expect(nov[0][0]).toBe('2026-10-26')
    expect(nov[0][6]).toBe('2026-11-01')
    expect(nov[5][6]).toBe('2026-12-06')
  })

  it('clamps a day into a range', () => {
    expect(clampDay('2026-12-01', null, '2026-11-25')).toBe('2026-11-25')
    expect(clampDay('2026-01-01', '2026-02-01', null)).toBe('2026-02-01')
    expect(clampDay('2026-05-05', null, null)).toBe('2026-05-05')
  })

  it('a short label: day and month', () => {
    expect(shortDay('2026-11-01')).toBe('1 Nov')
    expect(mediumDay('2026-11-01')).toBe('1 Nov 2026')
  })
})
