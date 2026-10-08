// Pure conversation logic: names, ordering, unread badges, mutes, read rule, sound decision.
import type { AppState } from './state'
import type { Channel } from './protocol/Channel'
import type { Message } from './protocol/Message'

export function isMuted(s: AppState, ch: Channel, nowIso: string): boolean {
  const active = (kind: 'server' | 'channel', id: string | null) =>
    !!id && s.mutes.some((m) => m.target_kind === kind && m.target_id === id && (m.until === null || m.until > nowIso))
  return active('channel', ch.id) || active('server', ch.server_id)
}

export function conversationName(s: AppState, channelId: string): string {
  const ch = s.channels[channelId]
  if (ch?.name) return ch.name
  const others = (s.dmMembers[channelId] ?? []).filter((u) => u !== s.me?.id)
  const names = others.map((u) => s.people[u]?.user.username ?? 'unknown')
  return names.join(', ') || 'Just you'
}

export function conversations(s: AppState, nowIso = new Date().toISOString()) {
  return Object.values(s.channels)
    .filter((c) => c.server_id === null && !s.hidden[c.id])
    .map((channel) => ({
      channel,
      name: conversationName(s, channel.id),
      members: s.dmMembers[channel.id] ?? [],
      latest: s.latest[channel.id] ?? null,
      unread: s.reads[channel.id]?.unread ?? 0,
      mentions: s.reads[channel.id]?.mentions ?? 0,
      muted: isMuted(s, channel, nowIso),
    }))
    .sort((a, b) => (b.latest?.id ?? b.channel.id).localeCompare(a.latest?.id ?? a.channel.id))
}

/** Home tab: unread + mentions of your DMs/groups (only mentions when muted). Server mentions show on server tabs. */
export function homeBadge(s: AppState, nowIso: string): number {
  let n = 0
  for (const [id, r] of Object.entries(s.reads)) {
    const ch = s.channels[id]
    if (ch?.server_id !== null || !ch) continue
    // A mention is also an unread message: count messages, not both (muted → only mentions).
    n += isMuted(s, ch, nowIso) ? r.mentions : r.unread
  }
  return n
}

export function serverBadge(s: AppState, serverId: string, nowIso: string): { dot: boolean; mentions: number } {
  let dot = false
  let mentions = 0
  for (const [id, r] of Object.entries(s.reads)) {
    const ch = s.channels[id]
    if (ch?.server_id !== serverId) continue
    mentions += r.mentions
    if (r.unread > 0 && !isMuted(s, ch, nowIso)) dot = true
  }
  return { dot, mentions }
}

export const shouldMarkRead = (v: { open: boolean; focused: boolean; atBottom: boolean }) => v.open && v.focused && v.atBottom

export const SOUND_GAP_MS = 2000

export function messageSound(v: {
  message: Message; me: string; channel: Channel; open: boolean; focused: boolean; muted: boolean; lastPlayedAt: number; now: number
}): boolean {
  const m = v.message
  if (m.kind !== 'normal' || m.author_id === v.me) return false
  if (v.open && v.focused) return false
  const mentioned = m.mentions.includes(v.me)
  const isPrivate = v.channel.server_id === null
  if (!isPrivate && !mentioned) return false
  if (v.muted && !mentioned) return false
  return v.now - v.lastPlayedAt >= SOUND_GAP_MS
}

/** "just now", "15m ago", "3h ago", "3d ago". */
export function relativeTime(iso: string, nowMs: number): string {
  const s = Math.max(0, (nowMs - Date.parse(iso)) / 1000)
  if (s < 60) return 'just now'
  if (s < 3600) return `${Math.floor(s / 60)}m ago`
  if (s < 86400) return `${Math.floor(s / 3600)}h ago`
  return `${Math.floor(s / 86400)}d ago`
}

/** One-line preview for the conversation list; `system` renders it muted/italic. */
export function previewText(s: AppState, m: Message): { text: string; system: boolean } {
  if (m.deleted) return { text: 'message deleted', system: true }
  if (m.kind === 'system') return { text: m.content, system: true }
  const line = m.content.replace(/\s+/g, ' ').trim()
  if (m.author_id === s.me?.id) return { text: `You: ${line}`, system: false }
  if (s.channels[m.channel_id]?.kind === 'group') {
    const who = (m.author_id && s.people[m.author_id]?.user.username) || 'someone'
    return { text: `${who}: ${line}`, system: false }
  }
  return { text: line, system: false }
}

/** Where the red NEW line goes: the first normal message after the read point that someone else wrote (-1 = none). */
export function firstUnreadIndex(messages: Message[], lastRead: string | null, me: string): number {
  return messages.findIndex((m) => (lastRead === null || m.id > lastRead) && m.kind === 'normal' && m.author_id !== me)
}

/** Tracks the open conversation; `removed` is true only when a channel we had seen disappears. */
export type OpenWatch = { id: string; seen: boolean }
export function watchOpen(prev: OpenWatch | null, id: string, exists: boolean): { state: OpenWatch; removed: boolean } {
  const seen = prev?.id === id ? prev.seen : false
  if (exists) return { state: { id, seen: true }, removed: false }
  return { state: { id, seen: false }, removed: seen }
}

/** How many people are in any of a server's voice channels (for the tab's speaker icon). */
export function inVoice(s: AppState, serverId: string): number {
  let n = 0
  for (const [id, members] of Object.entries(s.voice)) {
    if (s.channels[id]?.server_id === serverId) n += members.length
  }
  return n
}
