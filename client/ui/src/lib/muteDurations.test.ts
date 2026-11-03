import { describe, expect, it } from 'vitest'
import { MUTE_FOR, muteUntil } from './muteDurations'

describe('mute durations', () => {
  it('every_mute_menu_offers_the_same_lengths_shortest_first_then_until_turned_back_on', () => {
    expect(MUTE_FOR.map((d) => d.label)).toEqual(['For 15 minutes', 'For 1 hour', 'For 8 hours', 'For 24 hours', 'Until I turn it back on'])
  })

  it('a_length_ends_that_many_minutes_from_now_and_the_open_one_never_does', () => {
    const now = Date.parse('2026-11-03T10:00:00.000Z')
    expect(muteUntil(15, now)).toBe('2026-11-03T10:15:00.000Z')
    expect(muteUntil(24 * 60, now)).toBe('2026-11-04T10:00:00.000Z')
    expect(muteUntil(null, now)).toBeNull()
  })
})
