// F1 race weekends (#386), the pure parts: flags, countdown, times in the viewer's zone, picks.
import { describe, expect, it } from 'vitest'
import { countdown, driverShort, flagOf, ordinal, picksOpen, picksReady, placeLine, pointsText, predictionOf, randomPodium, sessionWhen } from './f1'
import type { F1Weekend } from './protocol/F1Weekend'

const weekend: F1Weekend = {
  season: 2030, round: 2, race_name: 'Harbour Grand Prix', circuit: 'Harbour Street Circuit', locality: 'Port Town', country: 'UK',
  sessions: [{ kind: 'race', starts_at: '2030-03-15T07:00:00.000Z' }], race_at: '2030-03-15T07:00:00.000Z',
}
const d = (id: string, code: string | null = null) => ({ id, code, name: `Driver ${id}`, team: null })

describe('F1 helpers', () => {
  it("flags come from Jolpica's country names", () => {
    expect(flagOf('UK')).toBe('🇬🇧')
    expect(flagOf('USA')).toBe('🇺🇸')
    expect(flagOf(' united arab emirates ')).toBe('🇦🇪')
    expect(flagOf('Monaco')).toBe('🇲🇨')
    expect(flagOf('Atlantis')).toBe('🏁')
  })

  it('the countdown reads in days and hours, then hours and minutes', () => {
    const at = Date.parse('2030-03-15T07:00:00Z')
    expect(countdown(at, at - (2 * 1440 + 4 * 60 + 5) * 60_000)).toBe('2d 4h')
    expect(countdown(at, at - (4 * 60 + 12) * 60_000)).toBe('4h 12m')
    expect(countdown(at, at - 12 * 60_000 - 5_000)).toBe('12m')
    expect(countdown(at, at - 30_000)).toBe('under a minute')
    expect(countdown(at, at)).toBeNull()
    expect(countdown(at, at + 1)).toBeNull()
  })

  it("sessions read in the viewer's own time", () => {
    expect(sessionWhen('2030-03-15T07:00:00.000Z', 'UTC')).toBe('Fri 15 Mar, 07:00')
    expect(sessionWhen('2030-03-15T23:30:00.000Z', 'Africa/Johannesburg')).toBe('Sat 16 Mar, 01:30')
  })

  it('picks are open until lights out', () => {
    const at = Date.parse(weekend.race_at)
    expect(picksOpen(weekend, at - 1)).toBe(true)
    expect(picksOpen(weekend, at)).toBe(false)
    expect(placeLine(weekend)).toBe('Harbour Street Circuit · Port Town, UK')
  })

  it('a podium is three different drivers; the dice pick three at random', () => {
    expect(picksReady(['a', 'b', 'c'])).toBe(true)
    expect(picksReady(['a', 'a', 'c'])).toBe(false)
    expect(picksReady(['a', null, 'c'])).toBe(false)
    expect(picksReady(['a', 'b'])).toBe(false)
    const field = ['a', 'b', 'c', 'e', 'f'].map((id) => d(id))
    let n = 0
    const rolled = randomPodium(field, () => [0.99, 0, 0.5][n++ % 3])
    expect(rolled).toEqual(['f', 'a', 'c'])
    expect(new Set(randomPodium(field)).size).toBe(3)
    expect(randomPodium(field.slice(0, 2))).toHaveLength(2)
  })

  it('drivers, places and points read short', () => {
    expect(driverShort(d('racer_a', 'RAA'))).toBe('RAA')
    expect(driverShort(d('racer_a'))).toBe('racer_a')
    expect(driverShort(undefined, 'x')).toBe('x')
    expect([1, 2, 3, 4, 11, 12, 13, 21, 22].map(ordinal)).toEqual(['1st', '2nd', '3rd', '4th', '11th', '12th', '13th', '21st', '22nd'])
    expect([pointsText(1), pointsText(5)]).toEqual(['1 pt', '5 pts'])
    expect(predictionOf([{ user_id: 'U1', picks: ['a', 'b', 'c'], points: null }], 'U1')?.picks).toEqual(['a', 'b', 'c'])
    expect(predictionOf([], 'U1')).toBeUndefined()
  })
})
