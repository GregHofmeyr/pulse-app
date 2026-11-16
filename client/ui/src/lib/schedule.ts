// Scheduled messages (#299) and message reminders (#300): their pure parts. The quick picks (each a time in the
// viewer's own time, turned into an instant), the custom time's check, how a time reads, and list order.
import type { MessageReminder } from './protocol/MessageReminder'
import type { ScheduledMessage } from './protocol/ScheduledMessage'

/** As the server's `schedule_limits` (protocol/src/rest.rs). */
export const SCHEDULE_LIMITS = { maxScheduled: 50, maxReminders: 100, maxDaysAhead: 365 } as const

export type Pick = { label: string; at: string }

const MIN = 60 * 1000
const HOUR = 60 * MIN
const DAY = 24 * HOUR

/** `now`'s day (or `days` later) at `hour`:00, the viewer's own time. */
function atHour(now: number, hour: number, days = 0): Date {
  const t = new Date(now)
  t.setDate(t.getDate() + days)
  t.setHours(hour, 0, 0, 0)
  return t
}

/** Send later's picks: in an hour, tonight at 20:00 (while that's more than half an hour off), tomorrow at 09:00. */
export function sendLaterPicks(now: number): Pick[] {
  const picks: Pick[] = [{ label: 'In 1 hour', at: new Date(now + HOUR).toISOString() }]
  const tonight = atHour(now, 20)
  if (tonight.getTime() - now > 30 * MIN) picks.push({ label: 'Tonight at 20:00', at: tonight.toISOString() })
  picks.push({ label: 'Tomorrow at 09:00', at: atHour(now, 9, 1).toISOString() })
  return picks
}

/** Remind me's picks: in 20 minutes, an hour, three hours, and tomorrow at 09:00. */
export function remindPicks(now: number): Pick[] {
  return [
    { label: 'In 20 minutes', at: new Date(now + 20 * MIN).toISOString() },
    { label: 'In 1 hour', at: new Date(now + HOUR).toISOString() },
    { label: 'In 3 hours', at: new Date(now + 3 * HOUR).toISOString() },
    { label: 'Tomorrow at 09:00', at: atHour(now, 9, 1).toISOString() },
  ]
}

/** A custom time's gate: one that isn't over and is at most [SCHEDULE_LIMITS.maxDaysAhead] days away. */
export function whenReady(at: string | null, now: number): at is string {
  if (!at) return false
  const t = Date.parse(at)
  return Number.isFinite(t) && t > now && t <= now + SCHEDULE_LIMITS.maxDaysAhead * DAY
}

const time = (t: Date) => t.toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit', hour12: false })

/** How a time reads, in the viewer's own time: "today at 20:00", "tomorrow at 09:00", "Mon 23 Nov at 09:00" (with the
 *  year when it isn't this one). */
export function whenLabel(iso: string, now: number): string {
  const t = new Date(iso)
  if (Number.isNaN(t.getTime())) return iso
  const day = (d: Date) => new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime()
  const n = new Date(now)
  const days = Math.round((day(t) - day(n)) / DAY)
  if (days === 0) return `today at ${time(t)}`
  if (days === 1) return `tomorrow at ${time(t)}`
  if (days === -1) return `yesterday at ${time(t)}`
  const date = t.toLocaleDateString(undefined, { weekday: 'short', day: 'numeric', month: 'short', ...(t.getFullYear() !== n.getFullYear() ? { year: 'numeric' } : {}) })
  return `${date} at ${time(t)}`
}

/** Scheduled messages soonest first (as the server lists them). */
export function sortScheduled(list: readonly ScheduledMessage[]): ScheduledMessage[] {
  return [...list].sort((a, b) => (a.send_at === b.send_at ? (a.id < b.id ? -1 : 1) : a.send_at < b.send_at ? -1 : 1))
}

/** `list` with `item` put in (or replaced), still in order. */
export function upsertScheduled(list: readonly ScheduledMessage[], item: ScheduledMessage): ScheduledMessage[] {
  return sortScheduled([...list.filter((x) => x.id !== item.id), item])
}

/** Reminders: those come due first (newest due first), then those still to come, soonest first. */
export function sortReminders(list: readonly MessageReminder[]): MessageReminder[] {
  return [...list].sort((a, b) => {
    if (a.due !== b.due) return a.due ? -1 : 1
    if (a.remind_at === b.remind_at) return a.id < b.id ? -1 : 1
    return (a.remind_at < b.remind_at) === !a.due ? -1 : 1
  })
}

export function upsertReminder(list: readonly MessageReminder[], item: MessageReminder): MessageReminder[] {
  return sortReminders([...list.filter((x) => x.id !== item.id), item])
}
