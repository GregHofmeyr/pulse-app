// What to draw for a person's presence (#45): the one place that reads `online`, `status` and `last_seen_at` for
// drawing. Every dot and "last seen" goes through presenceOf (presenceSites.test.ts pins the places).
import type { AppState } from './state'
import { ownStatus } from './status'

/** 'invisible' is only ever yours: others see an Invisible person exactly as offline. 'idle' (#53): Online with
 *  every device of theirs idle; never pickable, and DND and Invisible win over it. */
export type PresenceKind = 'online' | 'idle' | 'dnd' | 'offline' | 'invisible'
export type Presence = { kind: PresenceKind; label: string; lastSeen: string | null }

const LABEL: Record<PresenceKind, string> = { online: 'Online', idle: 'Idle', dnd: 'Do Not Disturb', offline: 'Offline', invisible: 'Invisible' }
const of = (kind: PresenceKind, lastSeen: string | null = null): Presence => ({ kind, label: LABEL[kind], lastSeen })

/** Yourself: your own status (a run-out DND timer is Online at `nowIso`). Anyone else: as the server lets you see
 *  them; an older server sends no status, so it goes by `online`. Someone unknown: offline, never seen.
 *  In a server (`serverId`, #154): someone's own status there, when they've set one (yours: as you set it). */
export function presenceOf(s: AppState, userId: string, nowIso: string, serverId: string | null = null): Presence {
  const p = s.people[userId]
  if (userId === s.me?.id) {
    // your own status in this server, where you've set one: Invisible shows as yours, the rest as they read
    const here = serverId ? s.myServerStatuses.find((x) => x.server_id === serverId)?.status : undefined
    // your own Idle shows too (#53), but only over Online: the server never sends it otherwise
    const own = here ?? ownStatus(s, nowIso)
    return of(own === 'online' && p?.idle ? 'idle' : own)
  }
  if (!p) return of('offline')
  const here = serverId ? p.in_servers?.find((x) => x.server_id === serverId) : undefined
  const status = here ? here.status : (p.status ?? (p.online ? 'online' : 'offline'))
  const idle = here ? !!here.idle : p.idle
  if (status === 'dnd') return of('dnd')
  if (status === 'online') return of(idle ? 'idle' : 'online')
  return of('offline', here ? here.last_seen_at : p.last_seen_at)
}

/** On: online, idle or DND (they're here, just maybe away or not to be disturbed). */
export const isOn = (p: Presence) => p.kind === 'online' || p.kind === 'idle' || p.kind === 'dnd'

/** Where a presence sorts among people (#53): active (online or DND) first, then idle, then everyone else. */
export const presenceRank = (p: Presence) => (p.kind === 'idle' ? 1 : isOn(p) ? 2 : 0)

/** A member list split as Discord does (#170): Online (DND included) above Offline, each in the list's order, with
 *  the idle (#53) at the end of Online in that order too. Goes
 *  only by what you're told: the server sends someone Invisible or hiding from you as offline, so they're Offline;
 *  so are you while Invisible, as everyone else sees you. */
export function byPresence<T>(s: AppState, items: readonly T[], idOf: (item: T) => string, nowIso: string, serverId: string | null = null): { online: T[]; offline: T[] } {
  const online: T[] = []
  const idle: T[] = []
  const offline: T[] = []
  for (const it of items) {
    const p = presenceOf(s, idOf(it), nowIso, serverId)
    ;(p.kind === 'idle' ? idle : isOn(p) ? online : offline).push(it)
  }
  return { online: [...online, ...idle], offline }
}
