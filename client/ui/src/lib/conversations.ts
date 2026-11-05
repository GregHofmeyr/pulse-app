// Pure conversation logic: names, ordering, unread badges, mutes, notification levels, read rule, sound decision.
import { countsAsUnread, type AppState } from './state'
import type { Channel } from './protocol/Channel'
import type { Message } from './protocol/Message'
import type { NotifyLevel } from './protocol/NotifyLevel'
import { displayName } from './selectors'
import { mentionLookup, showMentions } from './mentionStyles'
import { callLineText } from './calls'
import { isMediaHash, TOKEN_RE } from './emoji'

export function isMuted(s: AppState, ch: Channel, nowIso: string): boolean {
  const active = (kind: 'server' | 'channel', id: string | null) =>
    !!id && s.mutes.some((m) => m.target_kind === kind && m.target_id === id && (m.until === null || m.until > nowIso))
  return active('channel', ch.id) || active('server', ch.server_id)
}

/** Your level for a server (#195): All messages unless you picked another. */
export function serverLevel(s: AppState, serverId: string): NotifyLevel {
  return s.notifyLevels.find((l) => l.target_kind === 'server' && l.target_id === serverId)?.level ?? 'all'
}

/** A channel's own level, or null when it follows its server (#195). */
export function channelLevel(s: AppState, channelId: string): NotifyLevel | null {
  return s.notifyLevels.find((l) => l.target_kind === 'channel' && l.target_id === channelId)?.level ?? null
}

/** The level a server channel's messages alert at: its own, else its server's (#195). DMs and groups have none
 *  (`all`: every message alerts, subject to the mute). */
export function notifyLevel(s: AppState, ch: Channel): NotifyLevel {
  if (!ch.server_id) return 'all'
  return channelLevel(s, ch.id) ?? serverLevel(s, ch.server_id)
}

export function conversationName(s: AppState, channelId: string): string {
  const ch = s.channels[channelId]
  if (ch?.name) return ch.name
  const others = (s.dmMembers[channelId] ?? []).filter((u) => u !== s.me?.id)
  const names = others.map((u) => displayName(s, null, u))
  return names.join(', ') || 'Just you'
}

export function conversations(s: AppState, nowIso: string) {
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
    if (ch?.server_id !== null || !ch || s.hidden[id]) continue // closed: out of sight until a new message
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

/** Whether `message` is on screen: `open` says its channel is shown, `openThread` names the quote whose
 *  replies panel is open. A reply shows only in its panel (the channel's list never has it). */
export const onScreen = (v: { message: Message; open: boolean; openThread: string | null }) =>
  v.message.thread_root_id ? v.openThread === v.message.thread_root_id : v.open

export type AlertInput = {
  message: Message; me: string; channel: Channel; open: boolean; openThread: string | null; focused: boolean; muted: boolean
  /** The channel's notification level (#195, `notifyLevel`); a DM or group ignores it. */
  level: NotifyLevel
  /** Do Not Disturb is on (#45): nothing alerts, mentions included; unread and mention counts still count. */
  dnd?: boolean
}

/** Whether a new message is worth alerting you to (the message sound and the desktop notification):
 *  someone else's message in a DM or group, or in a server channel as its level says (#195: All messages, Only
 *  @mentions, or Nothing, not even mentions), unless you're reading it in the focused window. A mute lets only
 *  mentions through; Do Not Disturb lets nothing through. */
export function shouldAlert(v: AlertInput): boolean {
  if (v.dnd) return false
  const m = v.message
  if (m.kind !== 'normal' || m.deleted || m.author_id === v.me) return false
  if (onScreen(v) && v.focused) return false
  const mentioned = m.mentions.includes(v.me)
  const isPrivate = v.channel.server_id === null
  if (!isPrivate && (v.level === 'nothing' || (v.level === 'mentions' && !mentioned))) return false
  if (v.muted && !mentioned) return false
  return true
}

/** The message sound: an alert, at most one per [SOUND_GAP_MS]. */
export function messageSound(v: AlertInput & { lastPlayedAt: number; now: number }): boolean {
  return shouldAlert(v) && v.now - v.lastPlayedAt >= SOUND_GAP_MS
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
  if (m.call) return { text: callLineText(s, m), system: true }
  if (m.kind === 'system') return { text: m.content, system: true }
  // mentions as the names you see (never usernames, #47)
  const line = showMentions(m.content, mentionLookup(s), (id) => displayName(s, null, id)).replace(/\s+/g, ' ').trim()
  if (m.author_id === s.me?.id) return { text: `You: ${line}`, system: false }
  if (s.channels[m.channel_id]?.kind === 'group') {
    const who = m.author_id ? displayName(s, null, m.author_id) : 'someone'
    return { text: `${who}: ${line}`, system: false }
  }
  return { text: line, system: false }
}

export type PreviewPart = { text: string } | { emoji: { hash: string; name: string } }

/** A preview line in pieces: custom emoji tokens become the emoji's still picture (a sidebar never
 *  animates), and one the app doesn't know or that was deleted reads `:name:`. Neighbouring text
 *  joins up. */
export function previewParts(s: AppState, text: string): PreviewPart[] {
  const out: PreviewPart[] = []
  const add = (t: string) => {
    if (!t) return
    const last = out[out.length - 1]
    if (last && 'text' in last) last.text += t
    else out.push({ text: t })
  }
  let at = 0
  for (const m of text.matchAll(new RegExp(TOKEN_RE, 'g'))) {
    add(text.slice(at, m.index))
    at = m.index + m[0].length
    const e = s.customEmoji[m[3]]
    const hash = e?.still_hash ?? e?.hash
    if (hash && isMediaHash(hash)) out.push({ emoji: { hash, name: m[2] } })
    else add(`:${m[2]}:`)
  }
  add(text.slice(at))
  return out
}

/** Where the red NEW line goes: the first message after the read point that counts as unread (someone else's message or
 * call line, -1 = none). */
export function firstUnreadIndex(messages: Message[], lastRead: string | null, me: string): number {
  const i = messages.findIndex((m) => (lastRead === null || m.id > lastRead) && countsAsUnread(m, me))
  // Replying means you've caught up: no divider once one of your messages sits below it.
  return i >= 0 && messages.slice(i).some((m) => m.author_id === me) ? -1 : i
}

/** Where the NEW line is measured from. When you stop watching an open chat (window loses focus
 *  or you scroll up) you've seen everything up to the newest message, so it moves there and
 *  whatever arrives while you're away gets the line. */
export function reanchor(anchor: string | null, v: { was: boolean; now: boolean; newest: string | null }): string | null {
  return v.was && !v.now && v.newest ? v.newest : anchor
}

/** Tracks the open conversation; `removed` is true only when a channel we had seen disappears. */
export type OpenWatch = { id: string; seen: boolean }
export function watchOpen(prev: OpenWatch | null, id: string, exists: boolean): { state: OpenWatch; removed: boolean } {
  const seen = prev?.id === id ? prev.seen : false
  if (exists) return { state: { id, seen: true }, removed: false }
  return { state: { id, seen: false }, removed: seen }
}

/** How many people are in any of a server's voice channels (for the tab's speaker icon).
 *  Only for servers you've joined; elsewhere it's 0. */
export function inVoice(s: AppState, serverId: string): number {
  if (!(s.members[serverId] ?? []).some((m) => m.user.id === s.me?.id)) return 0
  let n = 0
  for (const [id, members] of Object.entries(s.voice)) {
    if (s.channels[id]?.server_id === serverId) n += members.length
  }
  return n
}
