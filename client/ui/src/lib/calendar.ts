// Days for the date picker (DatePicker.svelte): a day is 'YYYY-MM-DD' in your own time zone, the same string a
// native date input writes, so it compares as text and never shifts across midnight UTC.

const pad = (n: number) => String(n).padStart(2, '0')

/** A local date's day. */
export function toDay(d: Date): string {
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`
}

/** The start of a day, locally; null for anything that isn't a real day. */
export function parseDay(s: string): Date | null {
  const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(s)
  if (!m) return null
  const d = new Date(Number(m[1]), Number(m[2]) - 1, Number(m[3]))
  return toDay(d) === s ? d : null
}

/** The instant a local day starts. */
export function dayStartIso(day: string): string {
  return (parseDay(day) ?? new Date(NaN)).toISOString()
}

export function addDays(day: string, n: number): string {
  const d = parseDay(day)!
  return toDay(new Date(d.getFullYear(), d.getMonth(), d.getDate() + n))
}

/** The same day `n` months on, or that month's last day when it's shorter. */
export function addMonths(day: string, n: number): string {
  const d = parseDay(day)!
  const last = new Date(d.getFullYear(), d.getMonth() + n + 1, 0).getDate()
  return toDay(new Date(d.getFullYear(), d.getMonth() + n, Math.min(d.getDate(), last)))
}

/** A month (0-based) as 6 weeks of 7 days, Monday first: always 42 days, so the grid keeps one height. */
export function monthGrid(year: number, month: number): string[][] {
  const first = new Date(year, month, 1)
  const back = (first.getDay() + 6) % 7 // days since Monday
  return Array.from({ length: 6 }, (_, w) => Array.from({ length: 7 }, (_, i) => toDay(new Date(year, month, 1 - back + w * 7 + i))))
}

export function clampDay(day: string, min: string | null, max: string | null): string {
  if (max && day > max) return max
  if (min && day < min) return min
  return day
}

/** "1 Nov". */
export function shortDay(day: string): string {
  return new Intl.DateTimeFormat('en-GB', { day: 'numeric', month: 'short' }).format(parseDay(day)!)
}

/** "1 Nov 2026": a day field's button (DateField). */
export function mediumDay(day: string): string {
  return new Intl.DateTimeFormat('en-GB', { day: 'numeric', month: 'short', year: 'numeric' }).format(parseDay(day)!)
}
