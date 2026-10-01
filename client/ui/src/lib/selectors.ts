// Derived views over AppState. Pure, so they're tested without the UI.
import type { Channel } from './protocol/Channel'
import type { AppState } from './state'

const byPosition = (a: Channel, b: Channel) => a.position - b.position || (a.name ?? '').localeCompare(b.name ?? '')

export function channelsFor(s: AppState, serverId: string): { text: Channel[]; voice: Channel[] } {
  const mine = Object.values(s.channels).filter((c) => c.server_id === serverId)
  return {
    text: mine.filter((c) => c.kind === 'text').sort(byPosition),
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

export function typingNames(s: AppState, serverId: string | null, channelId: string, now: number): string[] {
  return Object.entries(s.typing[channelId] ?? {})
    .filter(([user, until]) => until > now && user !== s.me?.id)
    .map(([user]) => displayName(s, serverId, user))
}
