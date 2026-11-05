// What to draw for a person's presence (#45): the one place that reads `online`, `status` and `last_seen_at` for
// drawing. Every dot and "last seen" goes through presenceOf (presenceSites.test.ts pins the places).
import type { AppState } from './state'
import { ownStatus } from './status'

/** 'invisible' is only ever yours: others see an Invisible person exactly as offline. */
export type PresenceKind = 'online' | 'dnd' | 'offline' | 'invisible'
export type Presence = { kind: PresenceKind; label: string; lastSeen: string | null }

const LABEL: Record<PresenceKind, string> = { online: 'Online', dnd: 'Do Not Disturb', offline: 'Offline', invisible: 'Invisible' }
const of = (kind: PresenceKind, lastSeen: string | null = null): Presence => ({ kind, label: LABEL[kind], lastSeen })

/** Yourself: your own status (a run-out DND timer is Online at `nowIso`). Anyone else: as the server lets you see
 *  them; an older server sends no status, so it goes by `online`. Someone unknown: offline, never seen. */
export function presenceOf(s: AppState, userId: string, nowIso: string): Presence {
  if (userId === s.me?.id) return of(ownStatus(s, nowIso))
  const p = s.people[userId]
  if (!p) return of('offline')
  const status = p.status ?? (p.online ? 'online' : 'offline')
  if (status === 'dnd') return of('dnd')
  if (status === 'online') return of('online')
  return of('offline', p.last_seen_at)
}

/** On: online or DND (they're here, just maybe not to be disturbed). */
export const isOn = (p: Presence) => p.kind === 'online' || p.kind === 'dnd'
