import { describe, expect, it } from 'vitest'
import { emptyState, type AppState } from './state'
import { DND_FOR, dndActive, dndUntil, ownStatus, tomorrowMorning } from './status'
import type { OwnStatus } from './protocol/OwnStatus'

const withStatus = (myStatus: OwnStatus): AppState => ({ ...emptyState(), myStatus })

describe('DND timers (#45)', () => {
  it('the choices, in order', () => {
    expect(DND_FOR.map((c) => c.label)).toEqual(['For 1 hour', 'For 8 hours', 'Until tomorrow morning', 'Until I turn it off'])
  })
  it('tomorrow morning is 08:00 local time the next calendar day', () => {
    const at = (y: number, mo: number, d: number, h: number, mi = 0) => new Date(y, mo, d, h, mi)
    expect(tomorrowMorning(at(2026, 10, 5, 22, 30))).toEqual(at(2026, 10, 6, 8))
    expect(tomorrowMorning(at(2026, 10, 5, 1, 0))).toEqual(at(2026, 10, 6, 8))
    expect(tomorrowMorning(at(2026, 10, 5, 9, 0))).toEqual(at(2026, 10, 6, 8))
    // over a month and a year end
    expect(tomorrowMorning(at(2026, 10, 30, 12))).toEqual(at(2026, 11, 1, 8))
    expect(tomorrowMorning(at(2026, 11, 31, 23, 59))).toEqual(at(2027, 0, 1, 8))
  })
  it('each choice gives an absolute end time (or none)', () => {
    const now = new Date('2026-11-05T10:00:00Z')
    const by = (label: string) => DND_FOR.find((c) => c.label === label)!
    expect(dndUntil(by('For 1 hour'), now)).toBe('2026-11-05T11:00:00.000Z')
    expect(dndUntil(by('For 8 hours'), now)).toBe('2026-11-05T18:00:00.000Z')
    expect(dndUntil(by('Until tomorrow morning'), now)).toBe(tomorrowMorning(now).toISOString())
    expect(dndUntil(by('Until I turn it off'), now)).toBeNull()
  })
})

describe('dndActive / ownStatus', () => {
  const now = '2026-11-05T10:00:00.000Z'
  it('DND with no end is on until you turn it off', () => {
    const s = withStatus({ status: 'dnd', dnd_until: null })
    expect(dndActive(s, now)).toBe(true)
    expect(ownStatus(s, now)).toBe('dnd')
  })
  it('a timed DND is on until its end, then you are Online again', () => {
    const s = withStatus({ status: 'dnd', dnd_until: '2026-11-05T11:00:00.000Z' })
    expect(dndActive(s, now)).toBe(true)
    expect(dndActive(s, '2026-11-05T10:59:59.999Z')).toBe(true)
    expect(dndActive(s, '2026-11-05T11:00:00.000Z')).toBe(false)
    expect(ownStatus(s, '2026-11-05T11:00:00.000Z')).toBe('online')
  })
  it('online and invisible are never DND', () => {
    expect(dndActive(withStatus({ status: 'online', dnd_until: null }), now)).toBe(false)
    expect(dndActive(withStatus({ status: 'invisible', dnd_until: null }), now)).toBe(false)
    expect(ownStatus(withStatus({ status: 'invisible', dnd_until: null }), now)).toBe('invisible')
  })
})
