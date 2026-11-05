// Your own status (#45): the Do Not Disturb timers and whether DND is on right now. The server keeps the status and
// treats an expired DND as Online; the client flips back on time by itself (store.svelte.ts times it), no round trip.
import type { AppState } from './state'
import type { Status } from './protocol/Status'

/** How long DND lasts, like the mute timers (muteDurations.ts). `until`: minutes from now, 'morning' (08:00
 *  tomorrow, local) or null (until turned off). */
export type DndChoice = { label: string; until: number | 'morning' | null }
export const DND_FOR: readonly DndChoice[] = [
  { label: 'For 1 hour', until: 60 },
  { label: 'For 8 hours', until: 8 * 60 },
  { label: 'Until tomorrow morning', until: 'morning' },
  { label: 'Until I turn it off', until: null },
]

/** 08:00 local time on the next calendar day (also when it's 2 am: "tomorrow" is the next date). */
export function tomorrowMorning(now: Date): Date {
  return new Date(now.getFullYear(), now.getMonth(), now.getDate() + 1, 8, 0, 0, 0)
}

/** When a DND picked at `now` ends, as the absolute time the server takes (null: until turned off). */
export function dndUntil(choice: DndChoice, now: Date): string | null {
  if (choice.until === null) return null
  if (choice.until === 'morning') return tomorrowMorning(now).toISOString()
  return new Date(now.getTime() + choice.until * 60_000).toISOString()
}

/** DND is on: set, and its timer (if any) hasn't run out. */
export function dndActive(s: AppState, nowIso: string): boolean {
  const { status, dnd_until } = s.myStatus
  return status === 'dnd' && (dnd_until === null || Date.parse(dnd_until) > Date.parse(nowIso))
}

/** Your status as it stands now: a DND whose timer ran out is Online. */
export function ownStatus(s: AppState, nowIso: string): Status {
  if (s.myStatus.status === 'dnd') return dndActive(s, nowIso) ? 'dnd' : 'online'
  return s.myStatus.status
}
