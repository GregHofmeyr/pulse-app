// The app's single source of truth, as pure functions over plain data so they're easy to test.
// The reactive wrapper lives in store.svelte.ts.
import type { Channel } from './protocol/Channel'
import type { Event } from './protocol/Event'
import type { Member } from './protocol/Member'
import type { Message } from './protocol/Message'
import type { Mute } from './protocol/Mute'
import type { Person } from './protocol/Person'
import type { ReadState } from './protocol/ReadState'
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
  people: Record<string, Person> // userId -> person (presence)
  reads: Record<string, ReadState> // channelId -> your private read point + counts
  mutes: Mute[]
  hidden: Record<string, true> // closed conversations
  latest: Record<string, Message> // DM/group channelId -> newest message
}

export const TYPING_TTL_MS = 6000

export function emptyState(): AppState {
  return { me: null, servers: [], channels: {}, members: {}, dmMembers: {}, voice: {}, messages: {}, pending: {}, typing: {}, history: {}, conn: 'connecting', people: {}, reads: {}, mutes: [], hidden: {}, latest: {} }
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
    people: Object.fromEntries(r.people.map((p) => [p.user.id, p])),
    reads: Object.fromEntries(r.read_states.map((x) => [x.channel_id, x])),
    mutes: r.mutes,
    hidden: Object.fromEntries(r.hidden.map((id) => [id, true as const])),
    latest: Object.fromEntries(r.latest.map((m) => [m.channel_id, m])),
  }
}

/** Whether `channel` should have a read point for you: your DMs/groups, and text channels of servers you're in. */
function tracksUnread(s: AppState, channel: string): boolean {
  const ch = s.channels[channel]
  if (!ch || !s.me) return false
  if (ch.server_id === null) return true
  return ch.kind === 'text' && (s.members[ch.server_id] ?? []).some((m) => m.user.id === s.me!.id)
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
      // Conversations/servers you're in but have no read point for yet (created or joined since
      // Ready) start at zero, so their first messages count right away.
      const read = s.reads[c] ?? (tracksUnread(s, c) ? { channel_id: c, last_read_message_id: null, unread: 0, mentions: 0 } : undefined)
      // Others' normal messages count as unread (yours and system lines never do).
      const counts = read && m.kind === 'normal' && m.author_id !== s.me?.id
      const { [c]: _h, ...hidden } = s.hidden
      return {
        ...s,
        messages: { ...s.messages, [c]: upsertMessage(s.messages[c], m) },
        pending: nonce ? { ...s.pending, [c]: (s.pending[c] ?? []).filter((p) => p.nonce !== nonce) } : s.pending,
        typing: withoutTyping(s, c, m.author_id),
        latest: s.channels[c]?.server_id === null ? { ...s.latest, [c]: m } : s.latest,
        hidden,
        reads: counts
          ? {
              ...s.reads,
              [c]: { ...read, unread: read.unread + 1, mentions: read.mentions + (s.me && m.mentions.includes(s.me.id) ? 1 : 0) },
            }
          : s.reads,
      }
    }
    case 'MessageUpdated': {
      const m = e.d.message
      const c = m.channel_id
      const latest = s.latest[c]?.id === m.id ? { ...s.latest, [c]: m } : s.latest
      if (!s.messages[c]) return latest === s.latest ? s : { ...s, latest } // history never loaded
      return { ...s, latest, messages: { ...s.messages, [c]: upsertMessage(s.messages[c], m) } }
    }
    case 'MessageDeleted': {
      const { channel_id: c, message_id: id, author_id, mentions } = e.d
      const gone = (m: Message): Message => (m.id === id ? { ...m, deleted: true, content: '' } : m)
      // An unread message from someone else comes back out of the counts (mirrors MessageCreated).
      const read = s.reads[c]
      const wasUnread = read && author_id !== s.me?.id && id > (read.last_read_message_id ?? '')
      const mentioned = !!s.me && mentions.includes(s.me.id)
      return {
        ...s,
        messages: s.messages[c] ? { ...s.messages, [c]: s.messages[c].map(gone) } : s.messages,
        latest: s.latest[c] ? { ...s.latest, [c]: gone(s.latest[c]) } : s.latest,
        reads: wasUnread
          ? {
              ...s.reads,
              [c]: { ...read, unread: Math.max(0, read.unread - 1), mentions: Math.max(0, read.mentions - (mentioned ? 1 : 0)) },
            }
          : s.reads,
      }
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
    case 'UserCreated':
      return s.people[e.d.user.id] ? s : { ...s, people: { ...s.people, [e.d.user.id]: { user: e.d.user, online: false, last_seen_at: null } } }
    case 'PresenceChanged': {
      const p = s.people[e.d.user_id]
      if (!p) return s
      return { ...s, people: { ...s.people, [e.d.user_id]: { ...p, online: e.d.online, last_seen_at: e.d.last_seen_at ?? p.last_seen_at } } }
    }
    case 'GroupMembersChanged':
      return { ...s, dmMembers: { ...s.dmMembers, [e.d.channel_id]: e.d.user_ids } }
    case 'ChannelUpdated':
      return { ...s, channels: { ...s.channels, [e.d.channel.id]: e.d.channel } }
    case 'ChannelRemoved': {
      const c = e.d.channel_id
      const drop = <T,>(r: Record<string, T>): Record<string, T> => {
        const { [c]: _x, ...rest } = r
        return rest
      }
      return {
        ...s,
        channels: drop(s.channels),
        messages: drop(s.messages),
        dmMembers: drop(s.dmMembers),
        reads: drop(s.reads),
        latest: drop(s.latest),
        hidden: drop(s.hidden),
        pending: drop(s.pending),
        typing: drop(s.typing),
        history: drop(s.history),
      }
    }
    case 'ReadStateUpdated': {
      const c = e.d.channel_id
      const point = e.d.last_read_message_id
      // Messages newer than the point (they can race the event) stay unread.
      const after = (s.messages[c] ?? []).filter(
        (m) => (point === null || m.id > point) && m.kind === 'normal' && !m.deleted && m.author_id !== s.me?.id,
      )
      const me = s.me?.id ?? ''
      return {
        ...s,
        reads: {
          ...s.reads,
          [c]: { channel_id: c, last_read_message_id: point, unread: after.length, mentions: after.filter((m) => m.mentions.includes(me)).length },
        },
      }
    }
    case 'MutesChanged':
      return { ...s, mutes: e.d.mutes }
    case 'ConversationVisibility': {
      const { [e.d.channel_id]: _x, ...rest } = s.hidden
      return { ...s, hidden: e.d.hidden ? { ...rest, [e.d.channel_id]: true } : rest }
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
