// Custom status (#152): when each "clear after" ends, and whose status shows.
import { describe, expect, it } from 'vitest'
import { CLEAR_AFTER, clearAfterUntil, cleanText, customStatusOf, endOfToday, endOfWeek, myCustomStatus, statusWords, textLength } from './customStatus'
import { applyEvent, applyReady, emptyState } from './state'
import { testMe, testReady } from './testReady'
import type { Ready } from './protocol/Ready'

const sam = { id: 'U2', username: 'sam', display_name: 'sam', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
const choice = (id: string) => CLEAR_AFTER.find((c) => c.id === id)!

describe('clear after', () => {
  it('offers 30 minutes, 1 hour, 4 hours, today, this week and never', () => {
    expect(CLEAR_AFTER.map((c) => c.label)).toEqual(['30 minutes', '1 hour', '4 hours', 'Today', 'This week', "Don't clear"])
  })
  it('timed ones count from now; today ends at local midnight; never is null', () => {
    const now = new Date(2026, 10, 18, 15, 20) // a Wednesday afternoon, local
    expect(clearAfterUntil(choice('30m'), now)).toBe(new Date(now.getTime() + 30 * 60_000).toISOString())
    expect(clearAfterUntil(choice('4h'), now)).toBe(new Date(now.getTime() + 4 * 3_600_000).toISOString())
    expect(clearAfterUntil(choice('today'), now)).toBe(new Date(2026, 10, 19, 0, 0).toISOString())
    expect(clearAfterUntil(choice('never'), now)).toBeNull()
  })
  it('this week ends as Sunday does (weeks start on Monday), never more than 7 days off', () => {
    expect(endOfWeek(new Date(2026, 10, 18, 15, 20))).toEqual(new Date(2026, 10, 23)) // Wednesday → next Monday 00:00
    expect(endOfWeek(new Date(2026, 10, 22, 23, 0))).toEqual(new Date(2026, 10, 23)) // Sunday night → an hour later
    expect(endOfWeek(new Date(2026, 10, 16, 0, 0))).toEqual(new Date(2026, 10, 23)) // Monday midnight → a full week
    expect(endOfToday(new Date(2026, 11, 31, 9))).toEqual(new Date(2027, 0, 1))
  })
})

describe('whose status shows', () => {
  const NOW = '2026-11-16T10:00:00.000Z'
  const base = () => applyReady(emptyState(), testReady([], {
    people: [{ user: sam, online: true, last_seen_at: null, status: 'online', custom_status: { text: 'Gaming', emoji: '🎮' } }],
    my_custom_status: { current: { text: 'Lunch', emoji: null }, until: '2026-11-16T10:30:00.000Z', recent: [] },
  }))

  it("someone's as the server sent it; yours from your own copy, until it runs out", () => {
    const s = base()
    expect(customStatusOf(s, 'U2', NOW)).toEqual({ text: 'Gaming', emoji: '🎮' })
    expect(customStatusOf(s, testMe.id, NOW)).toEqual({ text: 'Lunch', emoji: null })
    expect(myCustomStatus(s, '2026-11-16T10:30:00.000Z')).toBeNull()
    expect(customStatusOf(s, 'U9', NOW)).toBeNull()
  })

  it('PresenceChanged carries it: absent is none, and offline drops it', () => {
    let s = base()
    s = applyEvent(s, { t: 'PresenceChanged', d: { user_id: 'U2', online: true, last_seen_at: null, status: 'dnd', custom_status: { text: 'Busy', emoji: null } } }, 0)
    expect(s.people.U2.custom_status).toEqual({ text: 'Busy', emoji: null })
    s = applyEvent(s, { t: 'PresenceChanged', d: { user_id: 'U2', online: true, last_seen_at: null, status: 'online' } }, 0)
    expect(customStatusOf(s, 'U2', NOW)).toBeNull()
    s = applyEvent(s, { t: 'PresenceChanged', d: { user_id: 'U2', online: true, last_seen_at: null, status: 'online', custom_status: { text: 'Back', emoji: null } } }, 0)
    s = applyEvent(s, { t: 'PresenceChanged', d: { user_id: 'U2', online: false, last_seen_at: NOW, status: 'offline', custom_status: { text: 'Back', emoji: null } } }, 0)
    expect(customStatusOf(s, 'U2', NOW)).toBeNull()
  })

  it('MyCustomStatusChanged is yours only', () => {
    let s = base()
    const status = { current: { text: 'Out', emoji: '🏖' }, until: null, recent: [{ text: 'Out', emoji: '🏖' }] }
    s = applyEvent(s, { t: 'MyCustomStatusChanged', d: { user_id: 'U2', status } }, 0)
    expect(s.myCustomStatus.current?.text).toBe('Lunch')
    s = applyEvent(s, { t: 'MyCustomStatusChanged', d: { user_id: testMe.id, status } }, 0)
    expect(s.myCustomStatus).toEqual(status)
  })

  it('an older server sends none', () => {
    const r = testReady([]) as Partial<Ready>
    delete r.my_custom_status
    expect(applyReady(emptyState(), r as Ready).myCustomStatus).toEqual({ current: null, until: null, recent: [] })
  })
})

describe('text', () => {
  it('is cleaned as the server cleans it, and counted in characters', () => {
    expect(cleanText('  a\n\tb ‮c‬  ')).toBe('a b c')
    expect(cleanText('x\u0007y')).toBe('xy')
    expect(textLength('👍é')).toBe(2)
  })
  it('reads as its text, or its emoji alone', () => {
    expect(statusWords({ text: 'Gaming', emoji: '🎮' })).toBe('Gaming')
    expect(statusWords({ text: '', emoji: '🎮' })).toBe('🎮')
    expect(statusWords({ text: '', emoji: 'custom:E1' }, (id) => (id === 'E1' ? 'pog' : undefined))).toBe(':pog:')
  })
})
