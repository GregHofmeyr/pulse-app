// F1 race weekends (#386), the pure parts: the country's flag, session names, times in the viewer's own time, the
// countdown, and podium predictions (who picked what, whether picks are still open, a random podium).
import type { F1Driver } from './protocol/F1Driver'
import type { F1Prediction } from './protocol/F1Prediction'
import type { F1SessionKind } from './protocol/F1SessionKind'
import type { F1Weekend } from './protocol/F1Weekend'

/** Jolpica's country names → ISO 3166 codes (every country F1 has raced in lately, and the names it uses for them). */
const COUNTRIES: Record<string, string> = {
  argentina: 'AR', australia: 'AU', austria: 'AT', azerbaijan: 'AZ', bahrain: 'BH', belgium: 'BE', brazil: 'BR',
  canada: 'CA', china: 'CN', france: 'FR', germany: 'DE', hungary: 'HU', india: 'IN', italy: 'IT', japan: 'JP',
  korea: 'KR', 'south korea': 'KR', malaysia: 'MY', mexico: 'MX', monaco: 'MC', morocco: 'MA', netherlands: 'NL',
  portugal: 'PT', qatar: 'QA', russia: 'RU', rwanda: 'RW', 'saudi arabia': 'SA', singapore: 'SG', 'south africa': 'ZA',
  spain: 'ES', sweden: 'SE', switzerland: 'CH', thailand: 'TH', turkey: 'TR', 'türkiye': 'TR', uae: 'AE',
  'united arab emirates': 'AE', uk: 'GB', 'united kingdom': 'GB', 'great britain': 'GB', usa: 'US',
  'united states': 'US', vietnam: 'VN',
}

/** The country's flag emoji; the chequered flag for one this doesn't know. */
export function flagOf(country: string): string {
  const code = COUNTRIES[country.trim().toLowerCase()]
  if (!code) return '🏁'
  return String.fromCodePoint(...[...code].map((c) => 0x1f1e6 + c.charCodeAt(0) - 65))
}

export const SESSION_LABELS: Record<F1SessionKind, string> = {
  practice1: 'Practice 1',
  practice2: 'Practice 2',
  practice3: 'Practice 3',
  sprint_qualifying: 'Sprint Qualifying',
  sprint: 'Sprint',
  qualifying: 'Qualifying',
  race: 'Race',
}

/** "Sat 21 Nov, 06:00" in the viewer's own time zone (`timeZone` for tests). */
export function sessionWhen(iso: string, timeZone?: string): string {
  const t = new Date(iso)
  const day = new Intl.DateTimeFormat('en-GB', { weekday: 'short', day: 'numeric', month: 'short', timeZone }).format(t)
  const time = new Intl.DateTimeFormat('en-GB', { hour: '2-digit', minute: '2-digit', hour12: false, timeZone }).format(t)
  return `${day.replace(',', '')}, ${time}`
}

/** Time left until `target` (ms) at `now` (ms): "2d 4h", "4h 12m", "12m", "under a minute"; `null` once it's here. */
export function countdown(target: number, now: number): string | null {
  const left = target - now
  if (!(left > 0)) return null
  const mins = Math.floor(left / 60_000)
  const d = Math.floor(mins / 1440)
  const h = Math.floor((mins % 1440) / 60)
  const m = mins % 60
  if (d > 0) return `${d}d ${h}h`
  if (h > 0) return `${h}h ${m}m`
  if (m > 0) return `${m}m`
  return 'under a minute'
}

/** The circuit line: "Las Vegas Strip Street Circuit · Las Vegas, USA". */
export function placeLine(w: F1Weekend): string {
  const where = [w.locality, w.country].filter(Boolean).join(', ')
  return where ? `${w.circuit} · ${where}` : w.circuit
}

/** When a card's picks close: its server's choice when the server says (#403: lights out, or qualifying's start),
 *  else lights out. */
export const picksCloseAt = (w: F1Weekend, closeAt?: string | null): string => closeAt ?? w.race_at

/** Whether a card's picks close before lights out (the server closes them at qualifying, #403). */
export const closesEarly = (w: F1Weekend, closeAt?: string | null): boolean => Date.parse(picksCloseAt(w, closeAt)) < Date.parse(w.race_at)

/** Podium picks are taken until they close: lights out, or qualifying's start by the server's choice (#403). */
export function picksOpen(w: F1Weekend, now: number, closeAt?: string | null): boolean {
  return Date.parse(picksCloseAt(w, closeAt)) > now
}

/** Your prediction on a card, if you made one. */
export function predictionOf(predictions: F1Prediction[], me: string): F1Prediction | undefined {
  return predictions.find((p) => p.user_id === me)
}

/** How a driver reads in a tight spot: their three letters, or their family name. */
export function driverShort(d: F1Driver | undefined, fallback = '?'): string {
  if (!d) return fallback
  return d.code ?? d.name.split(' ').slice(-1)[0] ?? d.name
}

/** A driver by id on a card. */
export const driverById = (drivers: F1Driver[], id: string): F1Driver | undefined => drivers.find((d) => d.id === id)

/** Three different drivers at random (fewer when the field is smaller): the dice next to the picks. */
export function randomPodium(drivers: F1Driver[], rand: () => number = Math.random): string[] {
  const pool = drivers.map((d) => d.id)
  const out: string[] = []
  while (out.length < 3 && pool.length) out.push(pool.splice(Math.floor(rand() * pool.length), 1)[0])
  return out
}

/** Whether a set of picks can be saved: three different drivers. */
export const picksReady = (picks: (string | null | undefined)[]): picks is string[] =>
  picks.length === 3 && picks.every((p) => !!p) && new Set(picks).size === 3

/** "1st", "2nd", "3rd", "4th"... */
export function ordinal(n: number): string {
  const tens = n % 100
  if (tens >= 11 && tens <= 13) return `${n}th`
  return `${n}${['th', 'st', 'nd', 'rd'][n % 10] ?? 'th'}`
}

/** A points total as people say it: "1 pt", "5 pts", "12.5 pts". */
export const pointsText = (n: number): string => `${n} ${n === 1 ? 'pt' : 'pts'}`
