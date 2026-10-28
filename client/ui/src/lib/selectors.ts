// Derived views over AppState. Pure, so they're tested without the UI.
import type { Channel } from './protocol/Channel'
import type { StreamInfo } from './protocol/StreamInfo'
import type { AppState } from './state'
import type { Message } from './protocol/Message'

const byPosition = (a: Channel, b: Channel) => a.position - b.position || (a.name ?? '').localeCompare(b.name ?? '')

export function channelsFor(s: AppState, serverId: string): { text: Channel[]; quotes: Channel[]; clips: Channel[]; voice: Channel[] } {
  const mine = Object.values(s.channels).filter((c) => c.server_id === serverId)
  return {
    text: mine.filter((c) => c.kind === 'text').sort(byPosition),
    quotes: mine.filter((c) => c.kind === 'quotes').sort(byPosition),
    clips: mine.filter((c) => c.kind === 'clips').sort(byPosition),
    voice: mine.filter((c) => c.kind === 'voice').sort(byPosition),
  }
}

export function isMember(s: AppState, serverId: string): boolean {
  return !!s.me && (s.members[serverId] ?? []).some((m) => m.user.id === s.me!.id)
}

/** Server nickname if set, else username. */
export function displayName(s: AppState, serverId: string | null, userId: string): string {
  if (serverId) {
    const m = (s.members[serverId] ?? []).find((x) => x.user.id === userId)
    if (m) return m.nickname ?? m.user.username
  }
  for (const list of Object.values(s.members)) {
    const m = list.find((x) => x.user.id === userId)
    if (m) return m.user.username
  }
  if (s.me?.id === userId) return s.me.username
  return 'unknown'
}

export type Occupant = { userId: string; name: string; muted: boolean; deafened: boolean }

export function voiceOccupants(s: AppState, serverId: string | null, channelId: string): Occupant[] {
  return (s.voice[channelId] ?? []).map((m) => ({
    userId: m.user_id,
    name: displayName(s, serverId, m.user_id),
    muted: m.flags.muted,
    deafened: m.flags.deafened,
  }))
}

export type SelfVoice = { channelId: string | null; joining: boolean; leaving: string | null }
export type RosterEntry = Occupant & { pending: boolean }

/** Who to show in a voice channel: the server's list, plus you straight away while joining and
 * minus you straight away after leaving. */
export function callRoster(s: AppState, serverId: string | null, channelId: string, me: SelfVoice): RosterEntry[] {
  const myId = s.me?.id
  let list: RosterEntry[] = voiceOccupants(s, serverId, channelId).map((o) => ({ ...o, pending: o.userId === myId && channelId === me.channelId && me.joining }))
  if (channelId === me.leaving && channelId !== me.channelId) list = list.filter((o) => o.userId !== myId)
  if (myId && s.me && channelId === me.channelId && !list.some((o) => o.userId === myId)) {
    list.push({ userId: myId, name: displayName(s, serverId, myId), muted: false, deafened: false, pending: me.joining })
  }
  return list
}

export function typingNames(s: AppState, serverId: string | null, channelId: string, now: number): string[] {
  return Object.entries(s.typing[channelId] ?? {})
    .filter(([user, until]) => until > now && user !== s.me?.id)
    .map(([user]) => displayName(s, serverId, user))
}

/** `prev` when `next` has the same items in the same order, else `next`: keeps a derived list's
 * identity across events that didn't change it, so components fed with it don't re-render. */
export function stableList<T>(prev: T[], next: T[]): T[] {
  return prev.length === next.length && prev.every((v, i) => v === next[i]) ? prev : next
}

/** When the next typing indicator in `channelId` runs out (ms timestamp), or null if nobody is typing. */
export function nextTypingExpiry(s: AppState, channelId: string, now: number): number | null {
  const live = Object.values(s.typing[channelId] ?? {}).filter((until) => until > now)
  return live.length ? Math.min(...live) : null
}

const NONE: Record<string, StreamInfo> = {}

/** The live streams in a voice channel, by streamer's user id. */
export function liveIn(s: AppState, channelId: string): Record<string, StreamInfo> {
  return s.streams[channelId] ?? NONE
}

export function isLive(s: AppState, channelId: string, userId: string): boolean {
  return !!s.streams[channelId]?.[userId]
}

/** Whether message `i` continues the previous one's group (same author, within 5 minutes, no inline reply). */
export function groupedWith(list: Message[], i: number): boolean {
  const m = list[i]
  const prev = list[i - 1]
  return !!prev && prev.author_id === m.author_id && prev.kind === 'normal' && !m.reply_to_id
    && new Date(m.created_at).getTime() - new Date(prev.created_at).getTime() < 5 * 60_000
}

/** Who can be @mentioned in a channel: a DM/group's members, or everyone for a server's channels (never me). */
export function mentionablesFor(s: AppState, channelId: string): { id: string; name: string }[] {
  const ids = s.channels[channelId]?.server_id === null ? (s.dmMembers[channelId] ?? []) : Object.keys(s.people)
  return ids
    .filter((id) => id !== s.me?.id)
    .map((id) => ({ id, name: s.people[id]?.user.username ?? '' }))
    .filter((p) => p.name)
}

/** A name lookup for one server's views (`state` reads the current state); no author is "Pulse". */
export function nameOfIn(state: () => AppState, serverId: string | null): (userId: string | null) => string {
  return (userId) => (userId ? displayName(state(), serverId, userId) : 'Pulse')
}
