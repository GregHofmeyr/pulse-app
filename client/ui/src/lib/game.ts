// "Playing" (#380): how someone's game reads, and its art. Pure. The server sends it only while they look online to
// you (Invisible, appear-offline-to and offline show none), and only what their switches allow, so whatever is in the
// state may be shown.
import { isMediaHash, mediaUrl } from './emoji'
import type { AppState } from './state'
import type { EmbedThumb } from './protocol/EmbedThumb'
import type { Playing } from './protocol/Playing'

/** What `userId` is playing, as the server lets you see it. */
export const playingOf = (s: AppState, userId: string): Playing | null => s.people[userId]?.playing ?? null

const url = (t: EmbedThumb | null | undefined): string | null => (t && isMediaHash(t.hash) ? mediaUrl(t.hash) : null)

/** The wide art behind a strip or a tile (the library hero, else the store header), and the logo; each null when the
 *  server has none (a non-Steam game, art switched off, or Steam has none). */
export function gameArt(p: Pick<Playing, 'art'>): { wide: string | null; logo: string | null } {
  return { wide: url(p.art?.hero) ?? url(p.art?.header), logo: url(p.art?.logo) }
}

/** How long they've played, from `started_at` to `nowMs`: "3m", "1h 05m", "less than a minute"; null with none (play
 *  time switched off) or an unreadable time. A start a little in the future (clocks apart) reads as just started. */
export function elapsedLabel(startedAt: string | null | undefined, nowMs: number): string | null {
  if (!startedAt) return null
  const t = Date.parse(startedAt)
  if (Number.isNaN(t)) return null
  const mins = Math.max(0, Math.floor((nowMs - t) / 60000))
  if (mins < 1) return 'less than a minute'
  if (mins < 60) return `${mins}m`
  return `${Math.floor(mins / 60)}h ${String(mins % 60).padStart(2, '0')}m`
}

/** "Playing Game". */
export const playingLine = (p: Playing): string => `Playing ${p.name}`
