// The app's single source of truth, as pure functions over plain data so they're easy to test.
// The reactive wrapper lives in store.svelte.ts.
import type { Channel } from './protocol/Channel'
import type { Event } from './protocol/Event'
import type { Member } from './protocol/Member'
import type { Message } from './protocol/Message'
import type { Ready } from './protocol/Ready'
import type { Server } from './protocol/Server'
import type { User } from './protocol/User'
import type { VoiceMember } from './protocol/VoiceMember'

export type ConnState = 'connecting' | 'connected' | 'reconnecting' | 'logged_out'

/** A message we've sent but the server hasn't confirmed yet (optimistic UI / offline outbox). */
export type Pending = { nonce: string; content: string; reply_to_id: string | null; status: 'pending' | 'failed' }

export type AppState = {
  me: User | null
  servers: Server[]
  channels: Record<string, Channel>
  members: Record<string, Member[]> // serverId -> members
  dmMembers: Record<string, string[]> // channelId -> userIds
  voice: Record<string, VoiceMember[]> // channelId -> who's in it
  messages: Record<string, Message[]> // channelId -> oldest..newest
  pending: Record<string, Pending[]> // channelId -> unsent
  typing: Record<string, Record<string, number>> // channelId -> userId -> expiresAt (ms)
  /** Which channels have had their latest page fetched (cleared on every Ready so we backfill). */
  history: Record<string, { loaded: boolean; start: boolean }>
  conn: ConnState
}

export const TYPING_TTL_MS = 6000

export function emptyState(): AppState {
  return { me: null, servers: [], channels: {}, members: {}, dmMembers: {}, voice: {}, messages: {}, pending: {}, typing: {}, history: {}, conn: 'connecting' }
}

export function applyReady(s: AppState, r: Ready): AppState {
  const channels = Object.fromEntries(r.channels.map((c) => [c.id, c]))
  // Keep history we've already paged in, for channels that still exist.
  const messages = Object.fromEntries(Object.entries(s.messages).filter(([id]) => id in channels))
  const pending = Object.fromEntries(Object.entries(s.pending).filter(([id]) => id in channels))
  return {
    ...s,
    me: r.me,
    servers: r.servers,
    channels,
    members: Object.fromEntries(r.members.map((m) => [m.server_id, m.members])),
    dmMembers: Object.fromEntries(r.dm_members.map((d) => [d.channel_id, d.user_ids])),
    voice: Object.fromEntries(r.voice.filter((v) => v.members.length > 0).map((v) => [v.channel_id, v.members])),
    messages,
    pending,
    typing: {},
    // Anything may have happened while we were away: refetch the latest page when a channel is viewed.
    history: {},
    conn: 'connected',
  }
}

function upsertMessage(list: Message[] | undefined, m: Message): Message[] {
  const cur = list ?? []
  const i = cur.findIndex((x) => x.id === m.id)
  if (i >= 0) return cur.map((x, j) => (j === i ? m : x))
  // ULIDs sort by time: insert in order (almost always at the end).
  const next = [...cur, m]
  next.sort((a, b) => (a.id < b.id ? -1 : a.id > b.id ? 1 : 0))
  return next
}

function withoutTyping(s: AppState, channel: string, user: string | null): AppState['typing'] {
  if (!user || !s.typing[channel]?.[user]) return s.typing
  const { [user]: _gone, ...rest } = s.typing[channel]
  return { ...s.typing, [channel]: rest }
}

export function applyEvent(s: AppState, e: Event, now: number): AppState {
  switch (e.t) {
    case 'MessageCreated': {
      const { message: m, nonce } = e.d
      const c = m.channel_id
      return {
        ...s,
        messages: { ...s.messages, [c]: upsertMessage(s.messages[c], m) },
        pending: nonce ? { ...s.pending, [c]: (s.pending[c] ?? []).filter((p) => p.nonce !== nonce) } : s.pending,
        typing: withoutTyping(s, c, m.author_id),
      }
    }
    case 'MessageUpdated': {
      const m = e.d.message
      if (!s.messages[m.channel_id]) return s // never loaded: nothing to update
      return { ...s, messages: { ...s.messages, [m.channel_id]: upsertMessage(s.messages[m.channel_id], m) } }
    }
    case 'MessageDeleted': {
      const { channel_id: c, message_id: id } = e.d
      const list = s.messages[c]
      if (!list) return s
      return { ...s, messages: { ...s.messages, [c]: list.map((m) => (m.id === id ? { ...m, deleted: true, content: '' } : m)) } }
    }
    case 'ChannelCreated':
      return { ...s, channels: { ...s.channels, [e.d.channel.id]: e.d.channel } }
    case 'ServerCreated':
      return s.servers.some((x) => x.id === e.d.server.id) ? s : { ...s, servers: [...s.servers, e.d.server] }
    case 'MemberJoined': {
      const list = s.members[e.d.server_id] ?? []
      if (list.some((m) => m.user.id === e.d.member.user.id)) return s
      return { ...s, members: { ...s.members, [e.d.server_id]: [...list, e.d.member] } }
    }
    case 'Typing': {
      const { channel_id: c, user_id: u } = e.d
      return { ...s, typing: { ...s.typing, [c]: { ...(s.typing[c] ?? {}), [u]: now + TYPING_TTL_MS } } }
    }
    case 'VoiceJoined': {
      const { channel_id: c, user_id: u, flags } = e.d
      const list = (s.voice[c] ?? []).filter((m) => m.user_id !== u)
      return { ...s, voice: { ...s.voice, [c]: [...list, { user_id: u, flags }] } }
    }
    case 'VoiceLeft': {
      const { channel_id: c, user_id: u } = e.d
      const list = (s.voice[c] ?? []).filter((m) => m.user_id !== u)
      const { [c]: _gone, ...rest } = s.voice
      return { ...s, voice: list.length ? { ...s.voice, [c]: list } : rest }
    }
    case 'VoiceStateChanged': {
      const { channel_id: c, user_id: u, flags } = e.d
      const list = s.voice[c]
      if (!list) return s
      return { ...s, voice: { ...s.voice, [c]: list.map((m) => (m.user_id === u ? { ...m, flags } : m)) } }
    }
  }
}

export function addPending(s: AppState, channel: string, p: Pending): AppState {
  return { ...s, pending: { ...s.pending, [channel]: [...(s.pending[channel] ?? []), p] } }
}

export function setPendingStatus(s: AppState, channel: string, nonce: string, status: Pending['status']): AppState {
  return { ...s, pending: { ...s.pending, [channel]: (s.pending[channel] ?? []).map((p) => (p.nonce === nonce ? { ...p, status } : p)) } }
}

/** Prepend an older page of history (from scroll-up). */
export function addHistory(s: AppState, channel: string, older: Message[]): AppState {
  let list = s.messages[channel] ?? []
  for (const m of older) list = upsertMessage(list, m)
  return { ...s, messages: { ...s.messages, [channel]: list } }
}

export function markHistory(s: AppState, channel: string, reachedStart: boolean): AppState {
  const prev = s.history[channel]
  return { ...s, history: { ...s.history, [channel]: { loaded: true, start: reachedStart || !!prev?.start } } }
}
