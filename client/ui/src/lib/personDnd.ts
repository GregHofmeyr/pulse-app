// Per-person Do Not Disturb (#310), the pure part: whose messages don't alert you (no message sound, no desktop
// notification), everywhere or only in DMs and groups, for a while or until you turn it off. Their messages still
// arrive and count as unread and as mentions; only the alert is held back. Kept by the server per person
// (state.personDnd); a timed one past its `until` counts as off here, at once, without waiting for the server.
import type { PersonDnd } from './protocol/PersonDnd'

/** `userId`'s entry while it's in force at `nowIso`, or null. */
export function personDndOf(list: readonly PersonDnd[], userId: string, nowIso: string): PersonDnd | null {
  const p = list.find((x) => x.user_id === userId)
  return p && (p.until === null || p.until > nowIso) ? p : null
}

/** The entries in force at `nowIso`, in the order they were added. */
export const activePersonDnd = (list: readonly PersonDnd[], nowIso: string): PersonDnd[] =>
  list.filter((p) => p.until === null || p.until > nowIso)

/** Whether a message from `authorId` in a chat (`isPrivate`: a DM or group) is held back from alerting you. */
export function quietFrom(list: readonly PersonDnd[], authorId: string | null, isPrivate: boolean, nowIso: string): boolean {
  if (!authorId) return false
  const p = personDndOf(list, authorId, nowIso)
  return !!p && (!p.dms_only || isPrivate)
}

/** "Until I turn it off", "Until 14:30", "Until Tue 09:00": how long it lasts, for the menu and Settings. */
export function personDndUntilLabel(until: string | null, nowMs: number): string {
  if (until === null) return 'Until you turn it off'
  const t = new Date(until)
  const time = t.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })
  const sameDay = new Date(nowMs).toDateString() === t.toDateString()
  return sameDay ? `Until ${time}` : `Until ${t.toLocaleDateString([], { weekday: 'short' })} ${time}`
}
