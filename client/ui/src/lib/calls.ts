// Calls in DMs and groups (#50), the pure part: how a call line reads for you, where a ring shows, and whether it may
// grab your attention. The ring store (calls.svelte.ts) and the components use these.
import type { AppState } from './state'
import type { CallRing } from './protocol/CallRing'
import type { Message } from './protocol/Message'
import { displayName } from './selectors'
import { isMuted } from './conversations'
import { dndActive } from './status'

/** How long a call lasted, to the nearest minute (at least one): "1 min", "12 min", "1 h 30 min", "2 h". */
export function callDuration(startIso: string, endIso: string): string {
  const mins = Math.max(1, Math.round((Date.parse(endIso) - Date.parse(startIso)) / 60_000))
  if (mins <= 60) return `${mins} min`
  const h = Math.floor(mins / 60)
  const m = mins % 60
  return m ? `${h} h ${m} min` : `${h} h`
}

/** A call line as you see it: "sam started a call", "… · 12 min" once it ended, "Missed call from sam" if it ended and
 * you never joined (never for your own call). Names are the ones you see people by. */
export function callLineText(s: AppState, m: Message): string {
  const call = m.call
  if (!call) return m.content
  const me = s.me?.id
  const mine = !!me && call.started_by === me
  const who = mine ? 'You' : call.started_by ? displayName(s, null, call.started_by) : 'Someone'
  if (!call.ended_at) return `${who} started a call`
  if (!mine && !(me && call.joined.includes(me))) return `Missed call from ${who}`
  return `${who} started a call · ${callDuration(m.created_at, call.ended_at)}`
}

/** The "is calling you" desktop notification's tag (#176): the call's own slot, so it never replaces (or is replaced by)
 * that conversation's message notifications, and can be taken back when the ring ends. A ULID makes 31 characters,
 * well inside Windows' 64. */
export const ringNoticeTag = (callId: string): string => `call-${callId}`

/** Inside the conversation the ring shows in its call band (no popover); anywhere else, as a popover. */
export function ringPlace(ring: CallRing, openChannelId: string | null): 'band' | 'popover' {
  return ring.channel_id === openChannelId ? 'band' : 'popover'
}

/** Whether a ring may grab your attention (popover, ringtone, OS notification): not under Do Not Disturb, and not in a
 * muted conversation. The band and the line show either way, so you can still join on purpose. */
export function ringAlerts(s: AppState, ring: CallRing, nowIso: string): boolean {
  if (dndActive(s, nowIso)) return false
  const ch = s.channels[ring.channel_id]
  return !(ch && isMuted(s, ch, nowIso))
}
