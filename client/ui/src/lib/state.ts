// The app's single source of truth, as pure functions over plain data so they're easy to test.
// The reactive wrapper lives in store.svelte.ts.
import type { Channel } from './protocol/Channel'
import type { CustomEmoji } from './protocol/CustomEmoji'
import type { Event } from './protocol/Event'
import type { Member } from './protocol/Member'
import type { Message } from './protocol/Message'
import type { Mute } from './protocol/Mute'
import type { Person } from './protocol/Person'
import type { QotyEvent } from './protocol/QotyEvent'
import type { ReadState } from './protocol/ReadState'
import type { Ready } from './protocol/Ready'
import type { Server } from './protocol/Server'
import type { SpeakerLink } from './protocol/SpeakerLink'
import type { StreamInfo } from './protocol/StreamInfo'
import type { User } from './protocol/User'
import type { VoiceMember } from './protocol/VoiceMember'

/** A winner card's text once its quote is deleted: the server writes exactly this (`quotes::on_delete`). */
export function deletedWinnerContent(year: number): string {
  return `Quote of the Year ${year}: a deleted quote`
}

export type ConnState =
  | 'connecting'
  | 'connected'
  | 'reconnecting'
  | 'too_many_devices'
  | 'logged_out'
  | 'update_required'
  | { rate_limited: number } // seconds until the next attempt

/** A message we've sent but the server hasn't confirmed yet (optimistic UI / offline outbox). */
export type Pending = { nonce: string; content: string; reply_to_id: string | null; /** The quote this answers (a reply lives in that quote's thread, not the channel list). */ thread_root_id: string | null; status: 'pending' | 'failed'; created_at: string }

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
  /** Which channels have had their latest page fetched (cleared on every Ready so we backfill,
   * except a detached window's). `end`: the list reaches the latest message (false for a window
   * into the past, from a jump). `missed`: live messages a detached list didn't take in (oldest
   * first, the newest MISSED_CAP); the list isn't at the latest until it holds those still there.
   * `missedMore`: older ones were dropped from `missed`, so one more page is fetched to be sure. */
  history: Record<string, History>
  conn: ConnState
  people: Record<string, Person> // userId -> person (presence)
  reads: Record<string, ReadState> // channelId -> your private read point + counts
  mutes: Mute[]
  hidden: Record<string, true> // closed conversations
  latest: Record<string, Message> // DM/group channelId -> newest message
  streams: Record<string, Record<string, StreamInfo>> // channelId -> userId -> live stream
  customEmoji: Record<string, CustomEmoji> // id -> emoji; deleted ones stay (hash null) so old messages render a chip
  frequentEmoji: string[] // your most-used reaction keys, best first
  threads: Record<string, Message[]> // quote id -> its replies, oldest..newest (replies never join `messages`)
  /** Quote id -> whether its fetched replies reach the thread's start. Present only once the thread was fetched
   * (live replies alone make `threads[root]` exist but don't set this); cleared on every Ready. */
  threadStart: Record<string, boolean>
  /** Quote id -> the newest reply you've seen, from ThreadSeen events (the root may not be loaded). */
  threadSeen: Record<string, string>
  /** Replies known deleted, so a repeated delete event can't lower a count twice. */
  goneReplies: Record<string, string> // reply id -> its channel
  qoty: Record<string, QotyEvent> // serverId -> its latest Quote of the Year vote (open, or ended and reopenable)
  speakerLinks: Record<string, SpeakerLink[]> // serverId -> plain names linked to members
}

export type History = { loaded: boolean; start: boolean; end: boolean; missed?: string[]; missedMore?: true }

/** Most missed live messages a detached list keeps track of. Pages are contiguous, so a list that
 * holds a kept one holds every older one too; the oldest are dropped first. */
export const MISSED_CAP = 200

export const TYPING_TTL_MS = 6000

export function emptyState(): AppState {
  return { me: null, servers: [], channels: {}, members: {}, dmMembers: {}, voice: {}, messages: {}, pending: {}, typing: {}, history: {}, conn: 'connecting', people: {}, reads: {}, mutes: [], hidden: {}, latest: {}, streams: {}, customEmoji: {}, frequentEmoji: [], threads: {}, threadStart: {}, threadSeen: {}, goneReplies: {}, qoty: {}, speakerLinks: {} }
}

export function applyReady(s: AppState, r: Ready): AppState {
  const channels = Object.fromEntries(r.channels.map((c) => [c.id, c]))
  // Keep history we've already paged in, for channels that still exist.
  const messages = Object.fromEntries(Object.entries(s.messages).filter(([id]) => id in channels))
  // A detached window (a jump into the past) stays as it is: it pages forward to the latest from its
  // bottom. Clearing its marker would merge the latest page into it, leaving a gap.
  const history = Object.fromEntries(Object.entries(s.history).filter(([id, h]) => id in channels && h.end === false))
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
    history,
    conn: 'connected',
    people: Object.fromEntries(r.people.map((p) => [p.user.id, p])),
    reads: Object.fromEntries(r.read_states.map((x) => [x.channel_id, x])),
    mutes: r.mutes,
    hidden: Object.fromEntries(r.hidden.map((id) => [id, true as const])),
    latest: Object.fromEntries(r.latest.map((m) => [m.channel_id, m])),
    streams: r.streams.reduce<AppState['streams']>((acc, x) => withStream(acc, x), {}),
    customEmoji: Object.fromEntries(r.custom_emoji.map((c) => [c.id, c])),
    frequentEmoji: r.frequent_emoji,
    // Replies may have changed while we were away: a panel refetches its thread.
    threads: {},
    threadStart: {},
    // Where you stand in each thread that has a counted mention for you ('' = nothing seen).
    threadSeen: Object.fromEntries(r.thread_seen.map((p) => [p.root_id, p.last_seen_id ?? ''])),
    goneReplies: {},
    qoty: latestQoty(r.qoty),
    speakerLinks: groupLinks(r.speaker_links),
  }
}

/** One vote per server: the most recently started. */
function latestQoty(events: QotyEvent[]): Record<string, QotyEvent> {
  const out: Record<string, QotyEvent> = {}
  for (const e of events) if (!out[e.server_id] || e.started_at >= out[e.server_id].started_at) out[e.server_id] = e
  return out
}

function groupLinks(links: SpeakerLink[]): Record<string, SpeakerLink[]> {
  const out: Record<string, SpeakerLink[]> = {}
  for (const l of links) (out[l.server_id] ??= []).push(l)
  return out
}

/** Whether `channel` should have a read point for you: your DMs/groups, and text and quotes channels of servers you're in. */
function tracksUnread(s: AppState, channel: string): boolean {
  const ch = s.channels[channel]
  if (!ch || !s.me) return false
  if (ch.server_id === null) return true
  return (ch.kind === 'text' || ch.kind === 'quotes') && (s.members[ch.server_id] ?? []).some((m) => m.user.id === s.me!.id)
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

// --- Quotes: replies live in `threads`, never in the channel list. ---

/** A quote as the event carries it, with what only we know put back: each line's `awarded` comes
 * from `awarders` (the event's flag is always false) and `replies_seen` is kept (events carry none). */
function withMyQuoteState(m: Message, old: Message | undefined, meId: string | undefined): Message {
  if (!m.quote) return m
  return {
    ...m,
    quote: {
      ...m.quote,
      lines: m.quote.lines.map((l) => ({ ...l, awarded: !!meId && l.awarders.includes(meId) })),
      replies_seen: old?.quote?.replies_seen ?? m.quote.replies_seen,
    },
  }
}

/** Change one quote wherever it is loaded in its channel's list. */
function mapQuote(s: AppState, channel: string, id: string, f: (m: Message) => Message): AppState {
  const list = s.messages[channel]
  if (!list?.some((m) => m.id === id)) return s
  return { ...s, messages: { ...s.messages, [channel]: list.map((m) => (m.id === id ? f(m) : m)) } }
}

/** A loaded message of `channel`: in its list, or a reply in a loaded thread. */
export function findMessage(s: AppState, channel: string, id: string): Message | undefined {
  const m = s.messages[channel]?.find((x) => x.id === id)
  if (m) return m
  for (const list of Object.values(s.threads)) {
    const r = list.find((x) => x.id === id && x.channel_id === channel)
    if (r) return r
  }
  return undefined
}

/** Change a loaded message of `channel` (list or thread); the state is returned as is when it isn't loaded. */
export function mapMessage(s: AppState, channel: string, id: string, f: (m: Message) => Message): AppState {
  const list = s.messages[channel]
  if (list?.some((x) => x.id === id)) return { ...s, messages: { ...s.messages, [channel]: list.map((x) => (x.id === id ? f(x) : x)) } }
  for (const [root, replies] of Object.entries(s.threads)) {
    if (replies.some((x) => x.id === id && x.channel_id === channel)) {
      return { ...s, threads: { ...s.threads, [root]: replies.map((x) => (x.id === id ? f(x) : x)) } }
    }
  }
  return s
}

/** Add a fetched page of a quote's replies (newest page, or older ones) and note whether the thread's start is reached. */
export function addReplies(s: AppState, root: string, replies: Message[], reachedStart: boolean): AppState {
  let list = s.threads[root] ?? []
  for (const m of replies) list = upsertMessage(list, m)
  return { ...s, threads: { ...s.threads, [root]: list }, threadStart: { ...s.threadStart, [root]: reachedStart || !!s.threadStart[root] } }
}

/** The greatest of some ids (`''` when there are none). */
export function maxIdOf(...ids: (string | null | undefined)[]): string {
  return ids.reduce<string>((a, b) => (b && b > a ? b : a), '')
}

/** A reply was created. Seen before (the outbox and the gateway both deliver it) it only refreshes the copy. */
function replyCreated(s: AppState, m: Message, nonce: string | null): AppState {
  const root = m.thread_root_id!
  const c = m.channel_id
  const known = !!s.threads[root]?.some((x) => x.id === m.id)
  const read = s.reads[c] ?? (tracksUnread(s, c) ? { channel_id: c, last_read_message_id: null, unread: 0, mentions: 0 } : undefined)
  const mentioned = !known && !!read && m.author_id !== s.me?.id && !!s.me && m.mentions.includes(s.me.id) && m.kind === 'normal'
  const next: AppState = {
    ...s,
    threads: { ...s.threads, [root]: upsertMessage(s.threads[root], m) },
    pending: nonce ? { ...s.pending, [c]: (s.pending[c] ?? []).filter((p) => p.nonce !== nonce) } : s.pending,
    typing: withoutTyping(s, c, m.author_id),
    // Replies are never unread; only a mention of me counts (until the thread is opened).
    reads: mentioned ? { ...s.reads, [c]: { ...read, mentions: read.mentions + 1 } } : s.reads,
  }
  if (known) return next
  return mapQuote(next, c, root, (q) => (q.quote ? { ...q, quote: { ...q.quote, reply_count: q.quote.reply_count + 1, last_reply_id: maxIdOf(q.quote.last_reply_id, m.id) } } : q))
}

function replyUpdated(s: AppState, m: Message): AppState {
  const root = m.thread_root_id!
  const list = s.threads[root]
  if (!list?.some((x) => x.id === m.id)) return s // thread not loaded: opening it fetches current copies
  return { ...s, threads: { ...s.threads, [root]: upsertEdited(list, m) } }
}

function replyDeleted(s: AppState, e: Extract<Event, { t: 'MessageDeleted' }>['d']): AppState {
  const { channel_id: c, message_id: id, thread_root_id: root, author_id, mentions } = e
  const list = s.threads[root!]
  const already = s.goneReplies[id] || (list?.find((x) => x.id === id)?.deleted ?? false)
  if (already) return s
  const rootMsg = s.messages[c]?.find((m) => m.id === root)
  const read = s.reads[c]
  // Counted on creation only for a normal reply from someone else (see replyCreated).
  const kind = list?.find((x) => x.id === id)?.kind ?? 'normal'
  const mentioned = !!s.me && mentions.includes(s.me.id) && author_id !== s.me.id && kind === 'normal'
  // Only a mention not yet cleared by opening the thread is still in the count.
  // Where the user's seen point is: from Ready or a ThreadSeen, the loaded quote, or (a reply that
  // arrived live this session) nothing seen yet. Unknown: Ready lists every thread with a counted
  // mention, so this one had none and the deleted mention was not in the count.
  const known = s.threadSeen[root!] !== undefined || !!rootMsg?.quote || !!list?.some((x) => x.id === id)
  const seen = [s.threadSeen[root!], rootMsg?.quote?.replies_seen].reduce<string>((a, b) => (b && b > a ? b : a), '')
  const counted = mentioned && !!read && known && id > seen
  const next: AppState = {
    ...s,
    goneReplies: { ...s.goneReplies, [id]: c },
    threads: list ? { ...s.threads, [root!]: list.map((x) => (x.id === id ? { ...x, deleted: true, content: '' } : x)) } : s.threads,
    reads: counted ? { ...s.reads, [c]: { ...read, mentions: Math.max(0, read.mentions - 1) } } : s.reads,
  }
  return mapQuote(next, c, root!, (q) => (q.quote ? { ...q, quote: { ...q.quote, reply_count: Math.max(0, q.quote.reply_count - 1) } } : q))
}

function awardChanged(s: AppState, d: Extract<Event, { t: 'QuoteAwardChanged' }>['d']): AppState {
  const mine = (a: string[]) => !!s.me && a.includes(s.me.id)
  return mapQuote(s, d.channel_id, d.message_id, (m) =>
    m.quote ? { ...m, quote: { ...m.quote, lines: m.quote.lines.map((l) => (l.id === d.line_id ? { ...l, awards: d.count, awarders: d.awarders, awarded: mine(d.awarders) } : l)) } } : m,
  )
}

function threadSeen(s: AppState, d: Extract<Event, { t: 'ThreadSeen' }>['d']): AppState {
  if (d.user_id !== s.me?.id) return s
  const read = s.reads[d.channel_id]
  const quoted = mapQuote(s, d.channel_id, d.root_id, (m) =>
    m.quote ? { ...m, quote: { ...m.quote, replies_seen: maxIdOf(m.quote.replies_seen, d.last_seen_id) } } : m,
  )
  const next = { ...quoted, threadSeen: { ...s.threadSeen, [d.root_id]: maxIdOf(s.threadSeen[d.root_id], d.last_seen_id) } }
  if (!read) return next
  // The server's absolute count wins; an older server sends only the delta.
  const mentions = d.mentions ?? Math.max(0, read.mentions - d.mentions_cleared)
  return mentions === read.mentions ? next : { ...next, reads: { ...next.reads, [d.channel_id]: { ...read, mentions } } }
}

function withoutTyping(s: AppState, channel: string, user: string | null): AppState['typing'] {
  if (!user || !s.typing[channel]?.[user]) return s.typing
  const { [user]: _gone, ...rest } = s.typing[channel]
  return { ...s.typing, [channel]: rest }
}

export function applyEvent(s: AppState, e: Event, now: number): AppState {
  switch (e.t) {
    case 'MessageCreated': {
      if (e.d.message.thread_root_id) return replyCreated(s, e.d.message, e.d.nonce)
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
        // A detached window shows the past: the message waits for the latest to be reloaded (no gap),
        // and is noted so paging forward doesn't stop short of it.
        messages: detached(s, c) ? s.messages : { ...s.messages, [c]: upsertMessage(s.messages[c], withMyQuoteState(m, undefined, s.me?.id)) },
        history: detached(s, c) ? { ...s.history, [c]: withMissed(s.history[c], m.id) } : s.history,
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
      if (e.d.message.thread_root_id) return replyUpdated(s, e.d.message)
      const m = e.d.message
      const c = m.channel_id
      const prev = s.latest[c]
      // Like the loaded copy (upsertEdited): the edit's reactions say me:false for everyone.
      const latest = prev?.id === m.id ? { ...s.latest, [c]: { ...m, reactions: prev.reactions } } : s.latest
      // History never loaded, or a detached window that doesn't hold it (adding it would leave a gap).
      if (!s.messages[c] || (detached(s, c) && !s.messages[c].some((x) => x.id === m.id))) return latest === s.latest ? s : { ...s, latest }
      return { ...s, latest, messages: { ...s.messages, [c]: upsertEdited(s.messages[c], withMyQuoteState(m, s.messages[c].find((x) => x.id === m.id), s.me?.id)) } }
    }
    case 'MessageDeleted': {
      if (e.d.thread_root_id) return replyDeleted(s, e.d)
      const { channel_id: c, message_id: id, author_id, mentions } = e.d
      // A deleted quote's winner cards (in its channel) drop its words now, as the server's
      // MessageUpdated for them will: a card showing a fetched copy of the quote needn't wait for it.
      const gone = (m: Message): Message =>
        m.id === id
          ? { ...m, deleted: true, content: '' }
          : m.qoty_card?.message_id === id && !m.deleted
            ? { ...m, content: deletedWinnerContent(m.qoty_card.year) }
            : m
      // An unread message from someone else comes back out of the counts (mirrors MessageCreated).
      const read = s.reads[c]
      const wasUnread = read && author_id !== s.me?.id && id > (read.last_read_message_id ?? '')
      const mentioned = !!s.me && mentions.includes(s.me.id)
      // A missed message that's gone (a hard delete, like a reopened vote's winner card) may never
      // come back in a page: stop waiting for it.
      const h = s.history[c]
      const history = h?.missed?.includes(id) ? { ...s.history, [c]: missing(h, h.missed.filter((x) => x !== id)) } : s.history
      return {
        ...s,
        history,
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
      // A thread goes with its channel, whether or not its quote is loaded.
      const roots = new Set((s.messages[c] ?? []).map((m) => m.id))
      for (const [root, list] of Object.entries(s.threads)) if (list.some((m) => m.channel_id === c)) roots.add(root)
      const without = <T,>(r: Record<string, T>) => Object.fromEntries(Object.entries(r).filter(([root]) => !roots.has(root)))
      return {
        ...s,
        threads: without(s.threads),
        threadStart: without(s.threadStart),
        threadSeen: without(s.threadSeen),
        goneReplies: Object.fromEntries(Object.entries(s.goneReplies).filter(([, channel]) => channel !== c)),
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
          // The server's absolute counts include mentions in quote replies and messages that aren't
          // loaded (a detached window); an older server sends none, and then only what is loaded can be counted.
          [c]: { channel_id: c, last_read_message_id: point, unread: e.d.unread ?? after.length, mentions: e.d.mentions ?? after.filter((m) => m.mentions.includes(me)).length },
        },
      }
    }
    case 'MutesChanged':
      return { ...s, mutes: e.d.mutes }
    case 'FrequentEmoji':
      return { ...s, frequentEmoji: e.d.frequent_emoji }
    case 'QuoteAwardChanged':
      return awardChanged(s, e.d)
    case 'ThreadSeen':
      return threadSeen(s, e.d)
    case 'SpeakerLinksChanged':
      return { ...s, speakerLinks: { ...s.speakerLinks, [e.d.server_id]: e.d.links } }
    case 'QotyStarted':
    case 'QotyUpdated':
    case 'QotyEnded':
      return { ...s, qoty: { ...s.qoty, [e.d.event.server_id]: e.d.event } }
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
      const loaded = !!findMessage(s, c, id) // not loaded: the fetch brings current reactions
      const inLatest = s.latest[c]?.id === id // the conversation preview keeps its own copy
      if (!loaded && !inLatest) return s
      const next = loaded ? mapMessage(s, c, id, apply) : s
      return { ...next, latest: inLatest ? { ...s.latest, [c]: apply(s.latest[c]) } : s.latest }
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
    thread_root_id: p.thread_root_id, quote: null, qoty_card: null,
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
  // A detached window is never trimmed: its newest messages aren't the channel's latest.
  if (!list || list.length <= keep || detached(s, channel)) return s
  // Never drop unread messages: the NEW line needs them (and an older server's ReadStateUpdated
  // carries no counts, so they're recounted from what's loaded). A channel with a read state but no read point is all unread: keep it whole.
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
 * history untouched. A page shorter than a full one marks the channel start as reached. */
export function addOlder(s: AppState, channel: string, before: string, older: Message[]): AppState {
  // Only prepends and trims change messages[0], so a different head means a trim (or newer page) intervened.
  if (s.messages[channel]?.[0]?.id !== before) return s
  // The start marker is set here, with the add, so a dropped page can't mark a trimmed list as "start reached".
  return markHistory(addHistory(s, channel, older), channel, older.length < PAGE)
}

/** Prepend an older page of history (from scroll-up). */
export function addHistory(s: AppState, channel: string, older: Message[]): AppState {
  let list = s.messages[channel] ?? []
  for (const m of older) list = upsertMessage(list, m)
  return { ...s, messages: { ...s.messages, [channel]: list } }
}

export function markHistory(s: AppState, channel: string, reachedStart: boolean): AppState {
  const prev = s.history[channel]
  return { ...s, history: { ...s.history, [channel]: { loaded: true, start: reachedStart || !!prev?.start, end: prev?.end ?? true } } }
}

// --- Detached windows (jump to a message): `history[c].end === false` means the list is a slice of
// the past, not the latest. Live messages aren't spliced into it (that would leave a gap), it is
// never trimmed, and paging forward (`appendNewer`) or reloading the latest attaches it again. ---

/** A full page of history, as the server sends it. */
export const PAGE = 50
/** An `around` window's share of a full page: older than the target, and newer (the server's split). */
const AROUND_OLDER = PAGE / 2
const AROUND_NEWER = PAGE - 1 - PAGE / 2

const detached = (s: AppState, channel: string) => s.history[channel]?.end === false

/** `h` waiting for `ids` (none: the field goes). */
function missing(h: History, ids: string[]): History {
  const { missed: _old, ...rest } = h
  return ids.length ? { ...rest, missed: ids } : rest
}

function withMissed(h: History, id: string): History {
  const ids = [...new Set([...(h.missed ?? []), id])].sort()
  if (ids.length <= MISSED_CAP) return missing(h, ids)
  return { ...missing(h, ids.slice(-MISSED_CAP)), missedMore: true }
}

/** Where `list` stands: at the latest when `end` and no missed live message newer than its newest
 * remains (a missed id it doesn't hold but is older than its newest is gone: pages are contiguous).
 * With older missed ids dropped, the first time it would be at the latest it fetches one more page. */
function reached(list: Message[], h: History | undefined, end: boolean): History {
  const newest = list[list.length - 1]?.id ?? ''
  const waiting = (h?.missed ?? []).filter((id) => id > newest)
  const more = !!h?.missedMore
  const at = end && !waiting.length && !more
  const base: History = { loaded: true, start: false, end: at }
  if (at) return base
  const next = missing(base, waiting)
  // Nothing else waiting at the end: the extra page is the next one asked for, so the flag goes.
  const extraNow = end && !waiting.length
  return more && !extraNow ? { ...next, missedMore: true } : next
}

function sorted(page: Message[]): Message[] {
  let list: Message[] = []
  for (const m of page) list = upsertMessage(list, m)
  return list
}

/** Replace `channel`'s list with the window `around` a message (a jump). A side shorter than its
 * share reached the channel's start or the latest. */
export function loadWindow(s: AppState, channel: string, page: Message[], around: string): AppState {
  const list = sorted(page)
  const start = list.filter((m) => m.id < around).length < AROUND_OLDER
  const end = list.filter((m) => m.id > around).length < AROUND_NEWER
  const h = { ...reached(list, s.history[channel], end), start }
  return { ...s, messages: { ...s.messages, [channel]: list }, history: { ...s.history, [channel]: h } }
}

/** Replace `channel`'s list with its latest page ("Jump to present"): attached again, unless a live
 * message newer than the page arrived meanwhile (then one `after` page fetches it). */
export function loadLatest(s: AppState, channel: string, page: Message[]): AppState {
  const list = sorted(page)
  const h = { ...reached(list, s.history[channel], true), start: list.length < PAGE }
  return { ...s, messages: { ...s.messages, [channel]: list }, history: { ...s.history, [channel]: h } }
}

/** Add a newer page fetched with `after` = the channel's newest message at the time (paging a
 * detached window forward). Dropped if the list changed meanwhile; a short page reaches the latest
 * (unless a live message it doesn't hold was missed meanwhile). */
export function appendNewer(s: AppState, channel: string, after: string, page: Message[]): AppState {
  const list = s.messages[channel]
  if (!list?.length || list[list.length - 1].id !== after) return s
  const h = s.history[channel]
  const next = addHistory(s, channel, page)
  const at = { ...reached(next.messages[channel], h, page.length < PAGE), start: !!h?.start }
  return { ...next, history: { ...s.history, [channel]: at } }
}

/** Called after a forward page after `after` (the channel's newest) came back empty a second time:
 * the missed messages in `stale` (those pending when that request was sent) and any dropped older
 * ones are not coming, so they are given up. A live message that arrived during the request is not
 * given up (that would leave a gap): the list stays detached waiting for it, and the caller pages on.
 * No-op when the list is no longer at `after` or is already attached. */
export function reachLatest(s: AppState, channel: string, after: string, stale: string[]): AppState {
  const list = s.messages[channel]
  const h = s.history[channel]
  if (!h || h.end || list?.[list.length - 1]?.id !== after) return s
  const { missedMore: _more, ...rest } = h
  const fresh = (h.missed ?? []).filter((id) => id > after && !stale.includes(id))
  const next = { ...missing(rest, fresh), loaded: true, end: !fresh.length }
  return { ...s, history: { ...s.history, [channel]: next } }
}
