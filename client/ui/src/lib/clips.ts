// Clips channels: pure helpers over AppState (feeds live in `state.ts`).
import type { AppState } from './state'
import type { ClipView } from './protocol/ClipView'
import type { Message } from './protocol/Message'

/** Which clips of a channel a feed shows: a game and/or an author (ids). */
export type ClipFilter = { game?: string; author?: string }

/** The key of a channel's feed for a filter (ids never contain `|`). */
export function feedKey(channelId: string, filter: ClipFilter): string {
  return `${channelId}|${filter.game ?? ''}|${filter.author ?? ''}`
}

/** Whether a clip message belongs in a feed with this filter. */
export function matchesFilter(m: Message, filter: ClipFilter): boolean {
  return (!filter.game || m.clip?.game_id === filter.game) && (!filter.author || m.author_id === filter.author)
}

/** How you look at a clips channel (the wall until you choose the list). */
export function clipView(s: AppState, channelId: string): ClipView {
  return s.clipViews[channelId] ?? 'wall'
}

/** "1:05", "0:09", "1:02:03". */
export function formatDuration(ms: number): string {
  const total = Math.max(0, Math.floor(ms / 1000))
  const h = Math.floor(total / 3600)
  const m = Math.floor((total % 3600) / 60)
  const sec = String(total % 60).padStart(2, '0')
  return h ? `${h}:${String(m).padStart(2, '0')}:${sec}` : `${m}:${sec}`
}

/** Decimal units: "512 B", "3 KB", "47.7 MB", "1.2 GB". */
export function formatBytes(n: number): string {
  if (n < 1000) return `${n} B`
  if (n < 1e6) return `${Math.round(n / 1e3)} KB`
  if (n < 1e9) return `${(n / 1e6).toFixed(1)} MB`
  return `${(n / 1e9).toFixed(1)} GB`
}

/** Whether a clip is newer than the read point: `seenAt` (the point captured when the channel was opened,
 * kept by the view), else your current one. */
export function isNewClip(s: AppState, channelId: string, messageId: string, seenAt?: string | null): boolean {
  const point = seenAt !== undefined ? seenAt : (s.reads[channelId]?.last_read_message_id ?? null)
  return point === null || messageId > point
}
