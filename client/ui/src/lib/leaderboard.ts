// Pure helpers for the quotes leaderboard (#410): its periods, which live events change it, a quote's lines as the
// board shows them, and the bars. No UI, no state changes.
import type { Event } from './protocol/Event'
import type { LeaderboardPeriod } from './protocol/LeaderboardPeriod'
import type { Message } from './protocol/Message'
import { speakerOf } from './quotes'
import type { AppState } from './state'

/** The period tabs, narrowest first. */
export const PERIODS: { value: LeaderboardPeriod; label: string; within: string }[] = [
  { value: 'week', label: 'Week', within: 'this week' },
  { value: 'month', label: 'Month', within: 'this month' },
  { value: 'year', label: 'Year', within: 'this year' },
  { value: 'all', label: 'All time', within: 'yet' },
]

/** The next wider period, for an empty board's "try a longer stretch" (none after all time). */
export function widerPeriod(p: LeaderboardPeriod): LeaderboardPeriod | null {
  const i = PERIODS.findIndex((x) => x.value === p)
  return PERIODS[i + 1]?.value ?? null
}

/** A tablist's arrow keys: the tab index `key` moves to from `at` among `n` (wrapping), or null for any other key. */
export function tabMove(key: string, at: number, n: number): number | null {
  switch (key) {
    case 'ArrowRight':
    case 'ArrowDown':
      return (at + 1) % n
    case 'ArrowLeft':
    case 'ArrowUp':
      return (at - 1 + n) % n
    case 'Home':
      return 0
    case 'End':
      return n - 1
    default:
      return null
  }
}

/** How long the board waits after the last relevant event before fetching again: a burst of reactions is one fetch. */
export const REFRESH_DEBOUNCE_MS = 800

/** Whether live event `e` (already applied to `s`) can change server `serverId`'s board: a quote posted, edited or
 *  deleted; a reaction on a quote; an award; a speaker link or a nickname (who a line counts for); a channel of it
 *  deleted or restored; a Quote of the Year ending or reopening. */
export function boardRefreshFor(e: Event, serverId: string, s: AppState): boolean {
  const inServer = (channelId: string) => s.channels[channelId]?.server_id === serverId
  const quotesChannel = (channelId: string) => inServer(channelId) && s.channels[channelId]?.kind === 'quotes'
  switch (e.t) {
    case 'MessageCreated':
    case 'MessageUpdated':
      return !!e.d.message.quote && inServer(e.d.message.channel_id)
    case 'MessageDeleted':
      return !e.d.thread_root_id && quotesChannel(e.d.channel_id)
    case 'ReactionAdded':
    case 'ReactionRemoved':
      return quotesChannel(e.d.channel_id)
    case 'QuoteAwardChanged':
      return inServer(e.d.channel_id)
    case 'SpeakerLinksChanged':
    case 'ChannelDeleted':
    case 'MemberJoined':
    case 'MemberUpdated':
      return e.d.server_id === serverId
    case 'ChannelRestored':
      return e.d.channel.server_id === serverId
    case 'QotyEnded':
    case 'QotyUpdated':
      return e.d.event.server_id === serverId
    default:
      return false
  }
}

/** One line of a quote as the board shows it: the words (or the picture's caption, or "Image") and who said it. */
export type BoardLine = { said: string; by: string; userId: string | null }

/** A quote's lines as the board shows them, in order (an empty list for a deleted quote). */
export function boardLines(s: AppState, serverId: string, m: Message): BoardLine[] {
  return (m.quote?.lines ?? []).map((l) => {
    const who = speakerOf(s, serverId, l)
    return { said: l.text ?? l.caption ?? 'Image', by: who.name, userId: who.userId }
  })
}

/** Each count's share of the largest, for the bars: 0..1, the leader at 1 (all 0 when every count is 0). */
export function shares(counts: number[]): number[] {
  const top = Math.max(0, ...counts)
  return counts.map((c) => (top > 0 ? c / top : 0))
}

/** Ranks for counts already sorted most first: tied counts share a rank ("1, 2, 2, 4"). */
export function ranks(counts: number[]): number[] {
  return counts.map((c) => counts.indexOf(c) + 1)
}
