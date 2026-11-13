import { describe, expect, it } from 'vitest'
import { PIN_FOR, livePin, pinFor, pinTip, pinnedNoun, pinsPlainMessages, pinsTitle, takesPins, timeLeft } from './pins'
import type { Message } from './protocol/Message'

const NOW = Date.parse('2026-11-09T10:00:00.000Z')
const iso = (ms: number) => new Date(NOW + ms).toISOString()
const H = 3_600_000

describe('pins (#178)', () => {
  it('offers the four lengths, shortest first, then forever', () => {
    expect(PIN_FOR.map((p) => p.label)).toEqual(['1 hour', '1 day', '1 week', 'Forever'])
    expect(PIN_FOR.map((p) => p.duration)).toEqual(['hour', 'day', 'week', 'forever'])
  })

  it('says how long is left in the largest whole unit', () => {
    expect(timeLeft(iso(50 * 60_000), NOW)).toBe('50 minutes left')
    expect(timeLeft(iso(60_000), NOW)).toBe('1 minute left')
    expect(timeLeft(iso(20_000), NOW)).toBe('1 minute left')
    expect(timeLeft(iso(H), NOW)).toBe('1 hour left')
    expect(timeLeft(iso(5 * H + 59 * 60_000), NOW)).toBe('5 hours left')
    expect(timeLeft(iso(24 * H), NOW)).toBe('1 day left')
    expect(timeLeft(iso(2 * 24 * H + H), NOW)).toBe('2 days left')
  })

  it('the marker tip names who pinned it and, for a timed pin, the time left', () => {
    expect(pinTip('alex', iso(2 * 24 * H + 3 * H), NOW)).toBe('Pinned by alex · 2 days left')
    expect(pinTip('alex', null, NOW)).toBe('Pinned by alex')
  })

  it('the line reads with its length, or without one for forever', () => {
    expect(pinFor('hour')).toBe('for 1 hour')
    expect(pinFor('day')).toBe('for 1 day')
    expect(pinFor('week')).toBe('for 1 week')
    expect(pinFor('forever')).toBe('')
  })

  it('a pin that has run out is no pin, before any event says so', () => {
    const m = { pin: { pinned_by: 'U1', pinned_at: iso(-H), expires_at: iso(1000) } } as Message
    expect(livePin(m, NOW)).toEqual(m.pin)
    expect(livePin(m, NOW + 1000)).toBeNull()
    expect(livePin({ pin: { pinned_by: 'U1', pinned_at: iso(-H), expires_at: null } } as Message, NOW + 1e12)).not.toBeNull()
    expect(livePin({} as Message, NOW)).toBeNull()
    expect(livePin({ deleted: true, pin: { pinned_by: 'U1', pinned_at: iso(0), expires_at: null } } as Message, NOW)).toBeNull()
  })
})

describe('pins in quotes and clips channels (#257)', () => {
  it('every chat takes pins; only chats offer Pin message on a plain message', () => {
    for (const k of ['text', 'voice', 'dm', 'group', 'quotes', 'clips'] as const) expect(takesPins(k)).toBe(true)
    expect(takesPins(undefined)).toBe(false)
    for (const k of ['text', 'voice', 'dm', 'group'] as const) expect(pinsPlainMessages(k)).toBe(true)
    // a quotes or clips channel's plain messages are thread replies
    expect(pinsPlainMessages('quotes')).toBe(false)
    expect(pinsPlainMessages('clips')).toBe(false)
  })

  it("a quotes channel's line pins a quote", () => {
    expect(pinnedNoun('quotes')).toBe('a quote')
    expect(pinnedNoun('text')).toBe('a message')
    expect(pinnedNoun('dm')).toBe('a message')
  })

  it('the pins are named for what they hold', () => {
    expect(pinsTitle('quotes')).toBe('Pinned quotes')
    expect(pinsTitle('clips')).toBe('Pinned clips')
    expect(pinsTitle('text')).toBe('Pinned messages')
    expect(pinsTitle('group')).toBe('Pinned messages')
  })
})
