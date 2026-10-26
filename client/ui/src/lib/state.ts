// The app's single source of truth, as pure functions over plain data so they're easy to test.
// The reactive wrapper lives in store.svelte.ts.
import type { Channel } from './protocol/Channel'
import type { CustomEmoji } from './protocol/CustomEmoji'
import type { Event } from './protocol/Event'
import type { Member } from './protocol/Member'
import type { Message } from './protocol/Message'
import type { Mute } from './protocol/Mute'
import type { Person } from './protocol/Person'
import type { ReadState } from './protocol/ReadState'
import type { Ready } from './protocol/Ready'
import type { Server } from './protocol/Server'
import type { StreamInfo } from './protocol/StreamInfo'
import type { User } from './protocol/User'
import type { VoiceMember } from './protocol/VoiceMember'

export type ConnState =
  | 'connecting'
  | 'connected'
  | 'reconnecting'
  | 'too_many_devices'
  | 'logged_out'
  | 'update_required'
  | { rate_limited: number } // seconds until the next attempt

/** A message we've sent but the server hasn't confirmed yet (optimistic UI / offline outbox). */
export type Pending = { nonce: string; content: string; reply_to_id: string | null; status: 'pending' | 'failed'; created_at: string }

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
  streams: Record<string, Record<string, StreamInfo>> // channelId -> userId -> live stream
  customEmoji: Record<string, CustomEmoji> // id -> emoji; deleted ones stay (hash null) so old messages render a chip
  frequentEmoji: string[] // your most-used reaction keys, best first
}

export const TYPING_TTL_MS = 6000

export function emptyState(): AppState {
  return { me: null, servers: [], channels: {}, members: {}, dmMembers: {}, voice: {}, messages: {}, pending: {}, typing: {}, history: {}, conn: 'connecting', people: {}, reads: {}, mutes: [], hidden: {}, latest: {}, streams: {}, customEmoji: {}, frequentEmoji: [] }
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
    streams: r.streams.reduce<AppState['streams']>((acc, x) => withStream(acc, x), {}),
    customEmoji: Object.fromEntries(r.custom_emoji.map((c) => [c.id, c])),
    frequentEmoji: r.frequent_emoji,
  }
}

/** Whether `channel` should have a read point for you: your DMs/groups, and text channels of servers you're in. */
function tracksUnread(s: AppState, channel: string): boolean {
  const ch = s.channels[channel]
  if (!ch || !s.me) return false
  if (ch.server_id === null) return true
  return ch.kind === 'text' && (s.members[ch.server_id] ?? []).some((m) => m.user.id === s.me!.id)
}

/** An edit event's reaction list is unreliable (`me` is always false), so reactions are kept as-is:
 * they change only through Reaction* events. */
function upsertEdited(list: Message[], m: Message): Message[] {
  const old = list.find((x) => x.id === m.id)
  return upsertMessage(list, old ? { ...m, reactions: old.reactions } : m)
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
      const prev = s.latest[c]
      // Like the loaded copy (upsertEdited): the edit's reactions say me:false for everyone.
      const latest = prev?.id === m.id ? { ...s.latest, [c]: { ...m, reactions: prev.reactions } } : s.latest
      if (!s.messages[c]) return latest === s.latest ? s : { ...s, latest } // history never loaded
      return { ...s, latest, messages: { ...s.messages, [c]: upsertEdited(s.messages[c], m) } }
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
    case 'FrequentEmoji':
      return { ...s, frequentEmoji: e.d.frequent_emoji }
    case 'ConversationVisibility': {
      const { [e.d.channel_id]: _x, ...rest } = s.hidden
      return { ...s, hidden: e.d.hidden ? { ...rest, [e.d.channel_id]: true } : rest }
    }
    // VoiceLeft deliberately leaves streams alone: the server follows it with StreamEnded.
    case 'StreamStarted':
    case 'StreamUpdated':
      return { ...s, streams: withStream(s.streams, e.d.stream) }
    case 'StreamEnded': {
      const { channel_id: c, user_id: u } = e.d
      const { [u]: _gone, ...rest } = s.streams[c] ?? {}
      if (!(u in (s.streams[c] ?? {}))) return s
      const { [c]: _ch, ...others } = s.streams
      return { ...s, streams: Object.keys(rest).length ? { ...others, [c]: rest } : others }
    }
    case 'ViewersChanged': {
      const cur = s.streams[e.d.channel_id]?.[e.d.user_id]
      if (!cur) return s
      return { ...s, streams: withStream(s.streams, { ...cur, viewers: e.d.viewers }) }
    }
    case 'EmojiCreated':
    case 'EmojiUpdated':
      return { ...s, customEmoji: { ...s.customEmoji, [e.d.emoji.id]: e.d.emoji } }
    case 'EmojiDeleted':
      return { ...s, customEmoji: { ...s.customEmoji, [e.d.emoji.id]: { ...e.d.emoji, hash: null, still_hash: null } } }
    case 'ReactionAdded':
    case 'ReactionRemoved': {
      const { channel_id: c, message_id: id, emoji, user_id, count } = e.d
      // The event carries the absolute count, so a reordered pair still ends on the latest value.
      const mine = user_id === s.me?.id
      const me = mine ? e.t === 'ReactionAdded' : undefined
      const apply = (m: Message): Message => {
        const has = m.reactions.some((r) => r.emoji === emoji)
        const reactions = (has ? m.reactions : [...m.reactions, { emoji, count, me: false }])
          .map((r) => (r.emoji === emoji ? { ...r, count, me: me ?? r.me } : r))
          .filter((r) => r.count > 0)
        return { ...m, reactions }
      }
      const list = s.messages[c]
      const loaded = !!list?.some((m) => m.id === id) // not loaded: the fetch brings current reactions
      const inLatest = s.latest[c]?.id === id // the conversation preview keeps its own copy
      if (!loaded && !inLatest) return s
      return {
        ...s,
        messages: loaded ? { ...s.messages, [c]: list!.map((m) => (m.id === id ? apply(m) : m)) } : s.messages,
        latest: inLatest ? { ...s.latest, [c]: apply(s.latest[c]) } : s.latest,
      }
    }
  }
}

function withStream(streams: AppState['streams'], x: StreamInfo): AppState['streams'] {
  return { ...streams, [x.channel_id]: { ...streams[x.channel_id], [x.user_id]: x } }
}

export function addPending(s: AppState, channel: string, p: Pending): AppState {
  return { ...s, pending: { ...s.pending, [channel]: [...(s.pending[channel] ?? []), p] } }
}

export function removePending(s: AppState, channel: string, nonce: string): AppState {
  const left = (s.pending[channel] ?? []).filter((p) => p.nonce !== nonce)
  const { [channel]: _gone, ...rest } = s.pending
  return { ...s, pending: left.length ? { ...s.pending, [channel]: left } : rest }
}

/** Unsent messages saved by an earlier run: all come back failed (never resent on their own). A
 * channel that is gone is skipped, and so is a nonce that is already pending. */
export function restoreUnsent(s: AppState, saved: Record<string, Pending[]>): AppState {
  const now = new Date().toISOString()
  const pending = { ...s.pending }
  let changed = false
  for (const [channel, list] of Object.entries(saved)) {
    if (!(channel in s.channels)) continue
    const known = new Set((pending[channel] ?? []).map((p) => p.nonce))
    // Failed, never resent: a message in flight at quit may already be on the server, so Retry can duplicate it.
    const fresh = list.filter((p) => !known.has(p.nonce)).map((p): Pending => ({ ...p, status: 'failed', created_at: p.created_at || now }))
    if (!fresh.length) continue
    pending[channel] = [...(pending[channel] ?? []), ...fresh]
    changed = true
  }
  return changed ? { ...s, pending } : s
}

/** An unsent message as a normal one (yours), so it renders through the same path as a sent one. */
export function pendingMessage(p: Pending, channelId: string, meId: string): Message {
  return {
    id: `pending:${p.nonce}`, channel_id: channelId, author_id: meId, kind: 'normal', content: p.content,
    reply_to_id: p.reply_to_id, created_at: p.created_at, edited_at: null, deleted: false, mentions: [], reactions: [],
  }
}

export function setPendingStatus(s: AppState, channel: string, nonce: string, status: Pending['status']): AppState {
  return { ...s, pending: { ...s.pending, [channel]: (s.pending[channel] ?? []).map((p) => (p.nonce === nonce ? { ...p, status } : p)) } }
}

/** Messages kept for a channel when you leave it: its two newest pages. */
export const KEEP_ON_LEAVE = 100

/** Keep only the newest `keep` messages of `channel` (on leaving it), so a long session doesn't
 * hold every message ever scrolled through. Older ones become fetchable again on scroll-up. */
export function trimHistory(s: AppState, channel: string, keep = KEEP_ON_LEAVE): AppState {
  const list = s.messages[channel]
  if (!list || list.length <= keep) return s
  // Never drop unread messages: ReadStateUpdated recounts unread from what's loaded, and the NEW
  // line needs them. A channel with a read state but no read point is all unread: keep it whole.
  const read = s.reads[channel]
  const firstUnread = !read
    ? -1
    : read.last_read_message_id === null
      ? 0
      : list.findIndex((m) => m.id > read.last_read_message_id!)
  const cut = firstUnread >= 0 ? Math.min(list.length - keep, firstUnread) : list.length - keep
  if (cut <= 0) return s
  const h = s.history[channel]
  return {
    ...s,
    messages: { ...s.messages, [channel]: list.slice(cut) },
    history: h ? { ...s.history, [channel]: { ...h, start: false } } : s.history,
  }
}

/** Add an older page fetched with `before` = the channel's oldest message at the time. Dropped if
 * the list was trimmed meanwhile (adding it would leave a hole that scroll-up never fills), with
 * history untouched. A page shorter than 50 marks the channel start as reached. */
export function addOlder(s: AppState, channel: string, before: string, older: Message[]): AppState {
  // Only prepends and trims change messages[0], so a different head means a trim (or newer page) intervened.
  if (s.messages[channel]?.[0]?.id !== before) return s
  // The start marker is set here, with the add, so a dropped page can't mark a trimmed list as "start reached".
  return markHistory(addHistory(s, channel, older), channel, older.length < 50)
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
