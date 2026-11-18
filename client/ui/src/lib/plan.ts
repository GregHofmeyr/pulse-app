// Plan something's pure parts (#387): `/plan`'s arguments (the command itself is in lib/commands.ts), a post's
// checks, the form's date and time fields, how a plan's time reads in the viewer's own time, and the forecast's words.
import type { Plan } from './protocol/Plan'
import type { PlanForecast } from './protocol/PlanForecast'
import type { Rsvp } from './protocol/Rsvp'

/** As the server's `plan_limits` (protocol/src/rest.rs). */
export const PLAN_LIMITS = { titleChars: 100, placeChars: 100, noteChars: 500, forecastDays: 16, remindMinutes: 60, maxHoursLong: 7 * 24 } as const

/** The answers, in the card's order, with their buttons' words. */
export const RSVPS: readonly { value: Rsvp; label: string }[] = [
  { value: 'going', label: 'Going' },
  { value: 'maybe', label: 'Maybe' },
  { value: 'cant', label: "Can't" },
]

const chars = (s: string) => [...s].length

/** What follows `/plan`: the title (cut to the limit). It only fills the plan form: nothing posts until Post plan. */
export const parsePlanArgs = (args: string): string => [...args.trim().replace(/\s+/g, ' ')].slice(0, PLAN_LIMITS.titleChars).join('')

/** The form's date and time fields (`YYYY-MM-DD`, `HH:MM`, the viewer's own time) as an instant (RFC 3339, UTC), or
 *  null when they don't make one. */
export function fieldsToIso(date: string, time: string): string | null {
  if (!/^\d{4}-\d{2}-\d{2}$/.test(date) || !/^\d{2}:\d{2}$/.test(time)) return null
  const [y, mo, d] = date.split('-').map(Number)
  const [h, mi] = time.split(':').map(Number)
  const t = new Date(y, mo - 1, d, h, mi)
  // a date that rolled over (the 31st of a 30-day month) isn't the one asked for
  if (Number.isNaN(t.getTime()) || t.getFullYear() !== y || t.getMonth() !== mo - 1 || t.getDate() !== d) return null
  return t.toISOString()
}

/** The form's optional end time (#399, `HH:MM`, blank = none) on the start's day, as an instant (RFC 3339, UTC); a
 *  time at or before the start's is the next day's (19:00 to 02:00 runs past midnight). Null when there's no end or
 *  the fields don't make one. */
export function endToIso(date: string, time: string, endTime: string): string | null {
  if (!endTime) return null
  const start = fieldsToIso(date, time)
  const end = fieldsToIso(date, endTime)
  if (!start || !end) return null
  if (Date.parse(end) > Date.parse(start)) return end
  const t = new Date(end)
  t.setDate(t.getDate() + 1)
  return t.toISOString()
}

const pad = (n: number) => String(n).padStart(2, '0')

/** An instant as the form's date and time fields, in the viewer's own time. */
export function isoToFields(iso: string): { date: string; time: string } {
  const t = new Date(iso)
  return { date: `${t.getFullYear()}-${pad(t.getMonth() + 1)}-${pad(t.getDate())}`, time: `${pad(t.getHours())}:${pad(t.getMinutes())}` }
}

/** The form's starting time: the next whole hour after an hour from `now`. */
export function defaultStart(now: number): { date: string; time: string } {
  const t = new Date(now + 60 * 60 * 1000)
  t.setMinutes(0, 0, 0)
  t.setHours(t.getHours() + 1)
  return isoToFields(t.toISOString())
}

/** Post plan's gate: a title within the limit, a time that isn't over, an end (when there is one) after it and
 *  within a week, and a place and note within theirs. */
export function planReady(title: string, startsAt: string | null, place: string, note: string, now: number, endsAt: string | null = null): boolean {
  const t = title.trim()
  if (!t || chars(t) > PLAN_LIMITS.titleChars) return false
  if (!startsAt || Date.parse(startsAt) < now - 5 * 60 * 1000) return false
  if (endsAt) {
    const long = Date.parse(endsAt) - Date.parse(startsAt)
    if (!(long > 0) || long > PLAN_LIMITS.maxHoursLong * 60 * 60 * 1000) return false
  }
  return chars(place.trim()) <= PLAN_LIMITS.placeChars && chars(note.trim()) <= PLAN_LIMITS.noteChars
}

const DAY = 24 * 60 * 60 * 1000
const startOfDay = (t: number) => {
  const d = new Date(t)
  d.setHours(0, 0, 0, 0)
  return d.getTime()
}

/** When a plan is, in the viewer's own time: "Today, 19:00", "Tomorrow, 19:00", "Fri 20 Nov, 19:00" (the year when
 *  it isn't this one). */
export function planWhen(iso: string, now: number): string {
  const t = Date.parse(iso)
  const time = new Intl.DateTimeFormat(undefined, { hour: '2-digit', minute: '2-digit' }).format(t)
  const days = Math.round((startOfDay(t) - startOfDay(now)) / DAY)
  if (days === 0) return `Today, ${time}`
  if (days === 1) return `Tomorrow, ${time}`
  const sameYear = new Date(t).getFullYear() === new Date(now).getFullYear()
  const day = new Intl.DateTimeFormat(undefined, { weekday: 'short', day: 'numeric', month: 'short', ...(sameYear ? {} : { year: 'numeric' }) }).format(t)
  return `${day}, ${time}`
}

/** When a plan runs, in the viewer's own time (#399): its start as [planWhen] says it, then its end when it has one:
 *  "Today, 19:00–22:00", or with the end's day when that's another: "Fri 20 Nov, 22:00 – Sat, 02:00". */
export function planRange(p: Plan, now: number): string {
  const start = planWhen(p.starts_at, now)
  if (!p.ends_at) return start
  const t = Date.parse(p.ends_at)
  const time = new Intl.DateTimeFormat(undefined, { hour: '2-digit', minute: '2-digit' }).format(t)
  if (startOfDay(t) === startOfDay(Date.parse(p.starts_at))) return `${start}–${time}`
  const day = new Intl.DateTimeFormat(undefined, { weekday: 'short' }).format(t)
  return `${start} – ${day}, ${time}`
}

/** Over: its end (#399) has passed. */
export const planEnded = (p: Plan, now: number) => !!p.ends_at && Date.parse(p.ends_at) <= now

/** Over: it has started (a plan stays answerable, but reads as past). */
export const planStarted = (p: Plan, now: number) => Date.parse(p.starts_at) <= now

/** Whether its day is within the forecast's reach (and it hasn't long started), so a forecast is worth asking for. */
export function inForecastWindow(p: Plan, now: number): boolean {
  const t = Date.parse(p.starts_at)
  return !!p.place && !p.cancelled_at && t > now - 12 * 60 * 60 * 1000 && t < now + PLAN_LIMITS.forecastDays * DAY
}

/** Your answer, from the card's lists. */
export function myRsvp(p: Plan, me: string): Rsvp | null {
  if (p.going.includes(me)) return 'going'
  if (p.maybe.includes(me)) return 'maybe'
  if (p.cant.includes(me)) return 'cant'
  return null
}

/** Your answer after a click on `pick`: it, or (on yours) none. */
export const nextRsvp = (mine: Rsvp | null, pick: Rsvp): Rsvp | null => (mine === pick ? null : pick)

/** Who answered `r`. */
export const answered = (p: Plan, r: Rsvp): string[] => (r === 'going' ? p.going : r === 'maybe' ? p.maybe : p.cant)

/** A WMO weather code in words, with its emoji. */
export function weatherOf(code: number): { emoji: string; text: string } {
  if (code === 0) return { emoji: '☀️', text: 'Clear' }
  if (code === 1) return { emoji: '🌤️', text: 'Mostly clear' }
  if (code === 2) return { emoji: '⛅', text: 'Partly cloudy' }
  if (code === 3) return { emoji: '☁️', text: 'Overcast' }
  if (code === 45 || code === 48) return { emoji: '🌫️', text: 'Fog' }
  if (code >= 51 && code <= 57) return { emoji: '🌦️', text: 'Drizzle' }
  if (code >= 61 && code <= 67) return { emoji: '🌧️', text: 'Rain' }
  if (code >= 71 && code <= 77) return { emoji: '❄️', text: 'Snow' }
  if (code >= 80 && code <= 82) return { emoji: '🌦️', text: 'Showers' }
  if (code === 85 || code === 86) return { emoji: '🌨️', text: 'Snow showers' }
  if (code >= 95) return { emoji: '⛈️', text: 'Thunderstorms' }
  return { emoji: '🌡️', text: 'Forecast' }
}

/** The forecast's line: "Rain · 22° at the start · 14°–25° · 70% chance of rain" (the temperature in the hour it
 *  starts when the server has it, #399). */
export function forecastLine(f: PlanForecast): string {
  const parts = [weatherOf(f.code).text]
  if (f.temp_at_start != null) parts.push(`${Math.round(f.temp_at_start)}° at the start`)
  parts.push(`${Math.round(f.temp_min)}°–${Math.round(f.temp_max)}°`)
  if (f.rain_chance != null && f.rain_chance > 0) parts.push(`${f.rain_chance}% chance of rain`)
  return parts.join(' · ')
}
