// Scheduled messages (#299) and reminders (#300): the quick picks, the custom time's check, how a time reads, and order.
import { describe, expect, it } from 'vitest'
import type { MessageReminder } from './protocol/MessageReminder'
import type { ScheduledMessage } from './protocol/ScheduledMessage'
import { remindPicks, sendLaterPicks, sortReminders, upsertScheduled, whenLabel, whenReady } from './schedule'

const at = (d: number, h: number, m = 0) => new Date(2026, 10, d, h, m).getTime()
const iso = (d: number, h: number, m = 0) => new Date(at(d, h, m)).toISOString()

describe('quick picks', () => {
  it('Send later: in an hour, tonight at 20:00 and tomorrow at 09:00, in your own time', () => {
    expect(sendLaterPicks(at(16, 10))).toEqual([
      { label: 'In 1 hour', at: iso(16, 11) },
      { label: 'Tonight at 20:00', at: iso(16, 20) },
      { label: 'Tomorrow at 09:00', at: iso(17, 9) },
    ])
  })

  it('tonight goes once 20:00 is less than half an hour off', () => {
    expect(sendLaterPicks(at(16, 19, 31)).map((p) => p.label)).toEqual(['In 1 hour', 'Tomorrow at 09:00'])
    expect(sendLaterPicks(at(16, 19, 29)).map((p) => p.label)).toContain('Tonight at 20:00')
  })

  it('Remind me: 20 minutes, an hour, three hours, tomorrow at 09:00 (across a month end too)', () => {
    expect(remindPicks(at(30, 23)).map((p) => p.at)).toEqual([iso(30, 23, 20), iso(31, 0), iso(31, 2), new Date(2026, 11, 1, 9).toISOString()])
  })
})

describe('a custom time', () => {
  it('must be ahead and within a year', () => {
    const now = at(16, 10)
    expect(whenReady(iso(16, 10, 1), now)).toBe(true)
    expect(whenReady(iso(16, 9, 59), now)).toBe(false)
    expect(whenReady(null, now)).toBe(false)
    expect(whenReady(new Date(now + 365 * 86400_000).toISOString(), now)).toBe(true)
    expect(whenReady(new Date(now + 366 * 86400_000).toISOString(), now)).toBe(false)
  })

  it('reads as today, tomorrow or a date', () => {
    const now = at(16, 10)
    expect(whenLabel(iso(16, 20), now)).toBe('today at 20:00')
    expect(whenLabel(iso(17, 9), now)).toBe('tomorrow at 09:00')
    expect(whenLabel(iso(15, 9), now)).toBe('yesterday at 09:00')
    expect(whenLabel(iso(23, 9), now)).toMatch(/23.*at 09:00$/)
    expect(whenLabel(new Date(2027, 0, 5, 9).toISOString(), now)).toMatch(/2027 at 09:00$/)
  })
})

describe('order', () => {
  const s = (id: string, send_at: string): ScheduledMessage => ({ id, channel_id: 'C1', content: id, reply_to_id: null, send_at, created_at: send_at, edited_at: null, failed: null })
  it('scheduled messages go soonest first, an edit moving one', () => {
    let list = upsertScheduled([], s('A', iso(17, 9)))
    list = upsertScheduled(list, s('B', iso(16, 20)))
    expect(list.map((x) => x.id)).toEqual(['B', 'A'])
    list = upsertScheduled(list, s('B', iso(18, 9)))
    expect(list.map((x) => x.id)).toEqual(['A', 'B'])
  })

  const r = (id: string, remind_at: string, due: boolean): MessageReminder => ({
    id, message_id: 'M', channel_id: 'C1', thread_root_id: null, remind_at, created_at: remind_at, due, seen: false,
    message: { message_id: 'M', author_id: null, content: '', deleted: false },
  })
  it('reminders come due first (newest first), then those to come (soonest first)', () => {
    const list = sortReminders([r('later', iso(18, 9), false), r('old', iso(15, 9), true), r('soon', iso(17, 9), false), r('new', iso(16, 9), true)])
    expect(list.map((x) => x.id)).toEqual(['new', 'old', 'soon', 'later'])
  })
})
