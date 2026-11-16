// Custom status (#152): a short line of text and an optional emoji beside a name. The server sends someone's only
// while they look online to you (Invisible, appear-offline-to and offline show none, like "Listening to"), so
// whatever is in the state may be shown; yours comes from your own sessions' copy, with when it clears. Pure.
import type { AppState } from './state'
import type { CustomStatus } from './protocol/CustomStatus'
import type { MyCustomStatus } from './protocol/MyCustomStatus'

/** The longest text, in characters, as the server counts them (code points). */
export const CUSTOM_STATUS_MAX = 128

export const NO_CUSTOM_STATUS: MyCustomStatus = { current: null, until: null, recent: [] }

/** When a custom status clears: minutes from now, the end of today or this week (local), or never. */
export type ClearAfter = { id: string; label: string; until: number | 'today' | 'week' | null }
export const CLEAR_AFTER: readonly ClearAfter[] = [
  { id: '30m', label: '30 minutes', until: 30 },
  { id: '1h', label: '1 hour', until: 60 },
  { id: '4h', label: '4 hours', until: 4 * 60 },
  { id: 'today', label: 'Today', until: 'today' },
  { id: 'week', label: 'This week', until: 'week' },
  { id: 'never', label: "Don't clear", until: null },
]

/** Midnight at the end of today, local. */
export function endOfToday(now: Date): Date {
  return new Date(now.getFullYear(), now.getMonth(), now.getDate() + 1, 0, 0, 0, 0)
}

/** Midnight at the end of this week's Sunday, local (weeks start on Monday). */
export function endOfWeek(now: Date): Date {
  const toMonday = ((8 - now.getDay()) % 7) || 7
  return new Date(now.getFullYear(), now.getMonth(), now.getDate() + toMonday, 0, 0, 0, 0)
}

/** When a custom status set at `now` clears, as the absolute time the server takes (null: never). */
export function clearAfterUntil(choice: ClearAfter, now: Date): string | null {
  if (choice.until === null) return null
  if (choice.until === 'today') return endOfToday(now).toISOString()
  if (choice.until === 'week') return endOfWeek(now).toISOString()
  return new Date(now.getTime() + choice.until * 60_000).toISOString()
}

/** Your custom status as it stands at `nowIso`: a run-out one is none (the server clears it within ~15 s). */
export function myCustomStatus(s: AppState, nowIso: string): CustomStatus | null {
  const { current, until } = s.myCustomStatus
  if (!current) return null
  return until && Date.parse(until) <= Date.parse(nowIso) ? null : current
}

/** Someone's custom status as you may see it; yours from your own copy. */
export function customStatusOf(s: AppState, userId: string, nowIso: string): CustomStatus | null {
  if (userId === s.me?.id) return myCustomStatus(s, nowIso)
  return s.people[userId]?.custom_status ?? null
}

/** The text in characters as the server counts them. */
export const textLength = (t: string): number => [...t].length

/** The text as the server will keep it: control and bidi characters gone, whitespace collapsed, trimmed. */
export function cleanText(t: string): string {
  // eslint-disable-next-line no-control-regex
  return t.replace(/\s+/g, ' ').replace(/[\u0000-\u001f\u007f-\u009f‪-‮⁦-⁩‎‏؜]/g, '').replace(/ +/g, ' ').trim()
}

/** The same status: equal text and emoji. */
export const sameStatus = (a: CustomStatus, b: CustomStatus): boolean => a.text === b.text && (a.emoji ?? null) === (b.emoji ?? null)

/** How a status reads in words, for labels and tooltips: the text, or the emoji alone. */
export function statusWords(c: CustomStatus, customName: (id: string) => string | undefined = () => undefined): string {
  if (c.text) return c.text
  if (!c.emoji) return ''
  return c.emoji.startsWith('custom:') ? `:${customName(c.emoji.slice(7)) ?? 'emoji'}:` : c.emoji
}
