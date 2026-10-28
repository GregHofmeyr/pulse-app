// Clips channels: pure helpers over AppState (feeds live in `state.ts`).
import type { AppState } from './state'
import type { ClipView } from './protocol/ClipView'
import type { Message } from './protocol/Message'
import type { Game } from './protocol/Game'
import { collapseName, nameKey } from './quotes'

/** Which clips of a channel a feed shows: a game and/or an author (ids). */
export type ClipFilter = { game?: string; author?: string }

/** The one way to build a filter: a missing, null or empty part is unset. */
export function clipFilter(game?: string | null, author?: string | null): ClipFilter {
  return { ...(game ? { game } : {}), ...(author ? { author } : {}) }
}

/** The key of a channel's feed for a filter (ids never contain `|`); every feed lookup goes through it. */
export function feedKey(channelId: string, filter: ClipFilter): string {
  const f = clipFilter(filter.game, filter.author)
  return `${channelId}|${f.game ?? ''}|${f.author ?? ''}`
}

/** A channel's feed for a filter, if started. */
export function feedOf(s: AppState, channelId: string, filter: ClipFilter): AppState['clipFeeds'][string] | undefined {
  return s.clipFeeds[feedKey(channelId, filter)]
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

/** The longest game name the server accepts (Unicode characters). */
const MAX_GAME_NAME = 40

/** The server's game for a detected title (same name rule as quotes), else the name to offer adding; null with no guess. */
export function matchGame(guess: string | null, games: Game[]): { game: Game } | { add: string } | null {
  const name = collapseName(guess ?? '')
  if (!name) return null
  const key = nameKey(name)
  const game = games.find((g) => nameKey(g.name) === key)
  return game ? { game } : { add: [...name].slice(0, MAX_GAME_NAME).join('').trimEnd() }
}

/** Whether a clip's thread has a reply newer than the one you last saw. */
export function hasNewClipReplies(m: Message): boolean {
  const c = m.clip
  return !!c?.last_reply_id && (c.replies_seen === null || c.last_reply_id > c.replies_seen)
}

/** The name of a clip's game in its server's registry (null when untagged or not known here). */
export function gameName(s: AppState, serverId: string, m: Message): string | null {
  const id = m.clip?.game_id
  return id ? ((s.games[serverId] ?? []).find((g) => g.id === id)?.name ?? null) : null
}
