// The app's single source of truth, as pure functions over plain data so they're easy to test.
// The reactive wrapper lives in store.svelte.ts.
import type { Channel } from './protocol/Channel'
import type { ClipFacets } from './protocol/ClipFacets'
import type { ClipView } from './protocol/ClipView'
import type { CustomEmoji } from './protocol/CustomEmoji'
import type { Event } from './protocol/Event'
import type { Game } from './protocol/Game'
import type { Member } from './protocol/Member'
import type { Message } from './protocol/Message'
import type { Mute } from './protocol/Mute'
import type { Person } from './protocol/Person'
import type { Profile } from './protocol/Profile'
import type { QotyEvent } from './protocol/QotyEvent'
import type { ReadState } from './protocol/ReadState'
import type { Ready } from './protocol/Ready'
import type { Server } from './protocol/Server'
import type { SpeakerLink } from './protocol/SpeakerLink'
import type { StreamInfo } from './protocol/StreamInfo'
import type { User } from './protocol/User'
import type { VoiceMember } from './protocol/VoiceMember'
import { feedKey, matchesFilter, type ClipFilter } from './clips'
import type { ClipJob } from './tauri'

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
  profiles: Record<string, Profile> // userId -> opened profile (cache)
  roleColors: Record<string, Record<string, string>> // serverId -> userId -> role colour (empty until the roles push)
  reads: Record<string, ReadState> // channelId -> your private read point + counts
  mutes: Mute[]
  hidden: Record<string, true> // closed conversations
  latest: Record<string, Message> // DM/group channelId -> newest message
  streams: Record<string, Record<string, StreamInfo>> // channelId -> userId -> live stream
  customEmoji: Record<string, CustomEmoji> // id -> emoji; deleted ones stay (hash null) so old messages render a chip
  frequentEmoji: string[] // your most-used reaction keys, best first
  /** Quote id -> its replies, oldest..newest (replies never join `messages`). A thread is held while its panel is
   * open (fetched, and live replies join it); otherwise only live mentions of you that are counted are kept, so
   * their delete can take them back out of the count. */
  threads: Record<string, Message[]>
  /** Quote id -> whether its fetched replies reach the thread's start. Present only while the thread is fetched
   * (live replies alone don't set this); cleared on every Ready and when its panel closes. */
  threadStart: Record<string, boolean>
  /** Quote id -> the newest reply you've seen, from ThreadSeen events (the root may not be loaded). */
  threadSeen: Record<string, string>
  /** Replies known deleted, so a repeated delete event can't lower a count twice. Kept while their quote is
   * loaded (trimming the quote away drops them). */
  goneReplies: Record<string, { channel: string; root: string }> // reply id -> where it was
  /** The quote whose replies panel is open, kept current by events like a loaded one, so the panel's pinned
   * copy stays live after the quote leaves its channel's list (a jump, "Jump to present"). */
  openRoot: Message | null
  qoty: Record<string, QotyEvent> // serverId -> its latest Quote of the Year vote (open, or ended and reopenable)
  speakerLinks: Record<string, SpeakerLink[]> // serverId -> plain names linked to members
  games: Record<string, Game[]> // serverId -> its games, by name
  clipViews: Record<string, ClipView> // channelId -> your view (absent = the wall)
  /** A clips channel's feeds by `feedKey` (cleared on every Ready: a viewed feed is fetched again). They are not
   * the text channel's `messages`/`history`; a clip's replies still live in `threads`. */
  clipFeeds: Record<string, ClipFeed>
  clipFacets: Record<string, ClipFacets> // channelId -> filter chips
  clipJobs: Record<string, ClipJob> // job id -> clip being sent (a cancelled one is removed; a posted one stays a moment)
}

export type ClipFeed = { channelId: string; filter: ClipFilter; items: Message[] /* newest first */; start: boolean; loading: boolean; /** Clips deleted while a page was being fetched (it may still carry them). */ gone?: string[] }

/** Clips per page of a feed (the server's page size). */
export const CLIP_PAGE = 30

export type History = { loaded: boolean; start: boolean; end: boolean; missed?: string[]; missedMore?: true }

/** Most missed live messages a detached list keeps track of. Pages are contiguous, so a list that
 * holds a kept one holds every older one too; the oldest are dropped first. */
export const MISSED_CAP = 200

export const TYPING_TTL_MS = 6000

export function emptyState(): AppState {
  return { me: null, servers: [], channels: {}, members: {}, dmMembers: {}, voice: {}, messages: {}, pending: {}, typing: {}, history: {}, conn: 'connecting', people: {}, profiles: {}, roleColors: {}, reads: {}, mutes: [], hidden: {}, latest: {}, streams: {}, customEmoji: {}, frequentEmoji: [], threads: {}, threadStart: {}, threadSeen: {}, goneReplies: {}, openRoot: null, qoty: {}, speakerLinks: {}, games: {}, clipViews: {}, clipFeeds: {}, clipFacets: {}, clipJobs: {} }
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
    qoty: Object.fromEntries(Object.entries(latestQoty(r.qoty)).map(([id, e]) => [id, withMyOptions(e, r.me.id, s.qoty[id])])),
    speakerLinks: groupLinks(r.speaker_links),
    games: r.games.reduce<AppState['games']>((acc, g) => withGame(acc, g), {}),
    clipViews: Object.fromEntries(r.clip_views.map((v) => [v.channel_id, v.view])),
    clipFeeds: {},
    clipFacets: {},
  }
}

/** One vote per server: the live one, else the most recently started. (A reopened vote can be
 * older than one started and ended since.) */
function latestQoty(events: QotyEvent[]): Record<string, QotyEvent> {
  const out: Record<string, QotyEvent> = {}
  const live = (e: QotyEvent) => e.ended_at === null
  for (const e of events) {
    const cur = out[e.server_id]
    if (!cur || live(e) > live(cur) || (live(e) === live(cur) && e.started_at >= cur.started_at)) out[e.server_id] = e
  }
  return out
}

function groupLinks(links: SpeakerLink[]): Record<string, SpeakerLink[]> {
  const out: Record<string, SpeakerLink[]> = {}
  for (const l of links) (out[l.server_id] ??= []).push(l)
  return out
}

/** Whether `channel` should have a read point for you: your DMs/groups, and text quotes and clips channels of servers you're in. */
export function tracksUnread(s: AppState, channel: string): boolean {
  const ch = s.channels[channel]
  if (!ch || !s.me) return false
  if (ch.server_id === null) return true
  return (ch.kind === 'text' || ch.kind === 'quotes' || ch.kind === 'clips') && (s.members[ch.server_id] ?? []).some((m) => m.user.id === s.me!.id)
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

// --- Quotes and clips: replies live in `threads`, never in the channel list. ---

type Thread = { reply_count: number; last_reply_id: string | null; replies_seen: string | null }

/** The reply counters of a message that has a thread (a quote or a clip). */
export function threadOf(m: Message): Thread | null {
  return m.quote ?? m.clip
}

/** `m` with its thread counters changed by `f` (as is when it has none). */
export function withThread(m: Message, f: (t: Thread) => Thread): Message {
  if (m.quote) return { ...m, quote: { ...m.quote, ...f(m.quote) } }
  if (m.clip) return { ...m, clip: { ...m.clip, ...f(m.clip) } }
  return m
}

function withGame(games: Record<string, Game[]>, g: Game): Record<string, Game[]> {
  const list = (games[g.server_id] ?? []).filter((x) => x.id !== g.id)
  return { ...games, [g.server_id]: [...list, g].sort((a, b) => a.name.localeCompare(b.name)) }
}

const newestFirst = (a: Message, b: Message) => (a.id < b.id ? 1 : a.id > b.id ? -1 : 0)

/** Change a clip in every feed that holds it (feeds without it are untouched). */
function mapFeedItems(s: AppState, channel: string, id: string, f: (m: Message) => Message): AppState {
  let feeds = s.clipFeeds
  for (const [k, feed] of Object.entries(s.clipFeeds)) {
    if (feed.channelId !== channel || !feed.items.some((m) => m.id === id)) continue
    feeds = { ...feeds, [k]: { ...feed, items: feed.items.map((m) => (m.id === id ? f(m) : m)) } }
  }
  return feeds === s.clipFeeds ? s : { ...s, clipFeeds: feeds }
}

/** The loaded copy of a message in its channel list or (for a clip) a feed. */
function loadedRoot(s: AppState, channel: string, id: string): Message | undefined {
  const m = s.messages[channel]?.find((x) => x.id === id)
  if (m) return m
  for (const feed of Object.values(s.clipFeeds)) {
    const f = feed.channelId === channel ? feed.items.find((x) => x.id === id) : undefined
    if (f) return f
  }
  return undefined
}

/** Start (or keep) the feed of a channel for a filter, ready to be filled by `loadFeedPage`. */
export function openFeed(s: AppState, channelId: string, filter: ClipFilter): AppState {
  const key = feedKey(channelId, filter)
  const feed = s.clipFeeds[key]
  if (feed) return feed.loading ? s : { ...s, clipFeeds: { ...s.clipFeeds, [key]: { ...feed, loading: true } } }
  return { ...s, clipFeeds: { ...s.clipFeeds, [key]: { channelId, filter, items: [], start: false, loading: true } } }
}

/** Add a fetched page of a feed (the newest, or one older than what is held: merged by id). Clips that arrived
 * live meanwhile stay, once each; ones deleted while it was in flight stay out; the start is reached on a short page. */
export function loadFeedPage(s: AppState, key: string, page: Message[]): AppState {
  const feed = s.clipFeeds[key]
  if (!feed) return s
  const seen = new Set(feed.items.map((m) => m.id))
  const gone = new Set(feed.gone)
  const items = [...feed.items, ...page.filter((m) => !seen.has(m.id) && !gone.has(m.id))].sort(newestFirst)
  return { ...s, clipFeeds: { ...s.clipFeeds, [key]: { ...feed, items, start: page.length < CLIP_PAGE, loading: false, gone: undefined } } }
}

/** A feed's fetch failed: it can be tried again. */
export function feedFailed(s: AppState, key: string): AppState {
  const feed = s.clipFeeds[key]
  return feed?.loading ? { ...s, clipFeeds: { ...s.clipFeeds, [key]: { ...feed, loading: false } } } : s
}

/** Keep a job while it is under way. A cancelled one is dropped; a posted one stays (`done`, with its
 * `message_id`) until `dropClipJob`, so its card can wait for the clip to arrive in its place. */
export function setClipJob(s: AppState, job: ClipJob): AppState {
  if (job.stage === 'done' && !job.message_id) return dropClipJob(s, job.id)
  return { ...s, clipJobs: { ...s.clipJobs, [job.id]: job } }
}

/** Let a job go. */
export function dropClipJob(s: AppState, id: string): AppState {
  if (!(id in s.clipJobs)) return s
  const { [id]: _gone, ...rest } = s.clipJobs
  return { ...s, clipJobs: rest }
}

/** A clip created, in every feed of its channel it matches. */
function clipCreated(s: AppState, m: Message): AppState {
  let feeds = s.clipFeeds
  for (const [k, feed] of Object.entries(s.clipFeeds)) {
    if (feed.channelId !== m.channel_id || !matchesFilter(m, feed.filter) || feed.items.some((x) => x.id === m.id)) continue
    feeds = { ...feeds, [k]: { ...feed, items: [...feed.items, m].sort(newestFirst) } }
  }
  return feeds === s.clipFeeds ? s : { ...s, clipFeeds: feeds }
}

/** A clip edited: replaced where it still matches, dropped where it no longer does, added where its edit now fits. Reactions and
 * the seen point are kept (events carry neither). */
function clipUpdated(s: AppState, m: Message): AppState {
  let feeds = s.clipFeeds
  for (const [k, feed] of Object.entries(s.clipFeeds)) {
    if (feed.channelId !== m.channel_id) continue
    const old = feed.items.find((x) => x.id === m.id)
    const fits = matchesFilter(m, feed.filter)
    // Not held yet: it belongs only where the feed reaches that far back (to its oldest clip, or its start).
    if (!old && !(fits && (feed.start || (feed.items.length > 0 && m.id > feed.items[feed.items.length - 1].id)))) continue
    const items = !fits
      ? feed.items.filter((x) => x.id !== m.id)
      : old
        ? feed.items.map((x) => (x.id === m.id ? { ...withMyQuoteState(m, old, undefined), reactions: old.reactions } : x))
        : [...feed.items, m].sort(newestFirst)
    feeds = { ...feeds, [k]: { ...feed, items } }
  }
  return feeds === s.clipFeeds ? s : { ...s, clipFeeds: feeds }
}

/** A quote as the event carries it, with what only we know put back: each line's `awarded` comes
 * from `awarders` (the event's flag is always false) and `replies_seen` is kept (events carry none). */
function withMyQuoteState(m: Message, old: Message | undefined, meId: string | undefined): Message {
  if (m.clip) return { ...m, clip: { ...m.clip, replies_seen: old?.clip?.replies_seen ?? m.clip.replies_seen } }
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

/** A vote as an event carries it: each option's quote through `withMyQuoteState` (the server
 * builds options for no one in particular). */
function withMyOptions(e: QotyEvent, meId: string | undefined, old: QotyEvent | undefined): QotyEvent {
  const before = old?.id === e.id ? old : undefined
  return { ...e, options: e.options.map((o) => ({ ...o, message: withMyQuoteState(o.message, before?.options.find((x) => x.message.id === o.message.id)?.message, meId) })) }
}

/** Change a quote wherever it is a QotY option (options are copies; edits, reactions and awards
 * reach them here). The state is returned as is when no vote holds it. */
function mapQotyMessages(s: AppState, id: string, f: (m: Message) => Message): AppState {
  let qoty = s.qoty
  for (const [server, e] of Object.entries(s.qoty)) {
    if (!e.options.some((o) => o.message.id === id)) continue
    qoty = { ...qoty, [server]: { ...e, options: e.options.map((o) => (o.message.id === id ? { ...o, message: f(o.message) } : o)) } }
  }
  return qoty === s.qoty ? s : { ...s, qoty }
}

/** Change the open replies panel's quote if it is `id` (the state is returned as is otherwise). */
function mapOpenRoot(s: AppState, id: string, f: (m: Message) => Message): AppState {
  return s.openRoot?.id === id ? { ...s, openRoot: f(s.openRoot) } : s
}

/** Change one quote or clip wherever it is loaded: in its channel's list or clip feeds, and as the open panel's root. */
function mapQuote(s: AppState, channel: string, id: string, f: (m: Message) => Message): AppState {
  const held = mapFeedItems(mapOpenRoot(s, id, f), channel, id, f)
  const list = s.messages[channel]
  if (!list?.some((m) => m.id === id)) return held
  return { ...held, messages: { ...s.messages, [channel]: list.map((m) => (m.id === id ? f(m) : m)) } }
}

/** Hold `root` as the open replies panel's quote (see `openRoot`). */
export function holdThread(s: AppState, root: Message): AppState {
  return s.openRoot === root ? s : { ...s, openRoot: root }
}

/** Its replies panel (in `channel`) closed: drop what was fetched for the thread (opening it again refetches),
 * keeping only live mentions of you that are still counted, and its gone replies once the quote isn't loaded
 * either. Another panel may hold its quote by now: that one stays held. */
export function closeThread(s: AppState, root: string, channel: string): AppState {
  const rootMsg = s.openRoot?.id === root ? s.openRoot : undefined
  const loaded = loadedRoot(s, channel, root)
  const copy = loaded ?? rootMsg
  const seen = maxIdOf(s.threadSeen[root], copy && threadOf(copy)?.replies_seen)
  const meId = s.me?.id
  const counted = (s.threads[root] ?? []).filter((m) => !!meId && m.kind === 'normal' && !m.deleted && m.author_id !== meId && m.mentions.includes(meId) && m.id > seen)
  const { [root]: _list, ...threads } = s.threads
  const { [root]: _start, ...threadStart } = s.threadStart
  return {
    ...s,
    threads: counted.length ? { ...threads, [root]: counted } : threads,
    threadStart,
    goneReplies: loaded ? s.goneReplies : Object.fromEntries(Object.entries(s.goneReplies).filter(([, g]) => g.root !== root)),
    openRoot: rootMsg ? null : s.openRoot,
  }
}

/** Unsent replies to a quote known deleted (loaded, or the open panel's) can never post: drop them. */
function withoutDeadReplies(s: AppState, channel: string): AppState {
  const list = s.pending[channel]
  if (!list?.some((p) => p.thread_root_id)) return s
  const dead = new Set((s.messages[channel] ?? []).filter((m) => m.deleted).map((m) => m.id))
  if (s.openRoot?.deleted && s.openRoot.channel_id === channel) dead.add(s.openRoot.id)
  const left = list.filter((p) => !p.thread_root_id || !dead.has(p.thread_root_id))
  if (left.length === list.length) return s
  const { [channel]: _gone, ...rest } = s.pending
  return { ...s, pending: left.length ? { ...s.pending, [channel]: left } : rest }
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
  if (list?.some((x) => x.id === id)) return mapOpenRoot({ ...s, messages: { ...s.messages, [channel]: list.map((x) => (x.id === id ? f(x) : x)) } }, id, f)
  for (const [root, replies] of Object.entries(s.threads)) {
    if (replies.some((x) => x.id === id && x.channel_id === channel)) {
      return { ...s, threads: { ...s.threads, [root]: replies.map((x) => (x.id === id ? f(x) : x)) } }
    }
  }
  return s
}

/** A message wherever it shows: its channel list, a thread, a clip feed or the open replies panel's root. */
export function findShown(s: AppState, channel: string, id: string): Message | undefined {
  return findMessage(s, channel, id) ?? loadedRoot(s, channel, id) ?? (s.openRoot?.id === id && s.openRoot.channel_id === channel ? s.openRoot : undefined)
}

/** `mapMessage`, also reaching a clip in its feeds and the open replies panel's root. */
export function mapShown(s: AppState, channel: string, id: string, f: (m: Message) => Message): AppState {
  const listed = mapMessage(s, channel, id, f)
  return listed !== s ? mapFeedItems(listed, channel, id, f) : mapQuote(s, channel, id, f)
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
  // A thread that isn't held can't say whether it has this reply. Replies arrive newest last, except
  // yours confirmed over REST (still pending until its first copy): any other no newer than the
  // quote's newest reply is a second copy, already counted.
  const rootMsg = loadedRoot(s, c, root) ?? (s.openRoot?.id === root ? s.openRoot : undefined)
  const rootQuote = rootMsg ? threadOf(rootMsg) : null
  const awaited = !!nonce && !!s.pending[c]?.some((p) => p.nonce === nonce)
  const held = !!s.threads[root]?.some((x) => x.id === m.id)
  const known = held || (!awaited && !!rootQuote?.last_reply_id && m.id <= rootQuote.last_reply_id)
  const read = s.reads[c] ?? (tracksUnread(s, c) ? { channel_id: c, last_read_message_id: null, unread: 0, mentions: 0 } : undefined)
  // A counted mention is always kept in its thread (below), so only the thread copy says it was
  // counted: a page can bring the quote with this reply as its newest before the reply itself arrives.
  // One at or before my seen point (the thread was opened elsewhere) was never counted.
  const seen = [s.threadSeen[root], rootQuote?.replies_seen].reduce<string>((a, b) => (b && b > a ? b : a), '')
  const mentioned = !held && m.id > seen && !!read && m.author_id !== s.me?.id && !!s.me && m.mentions.includes(s.me.id) && m.kind === 'normal'
  // Only an open panel's thread (fetched, or its first page on the way) takes live replies; a counted
  // mention is kept too, for its delete.
  const keep = root in s.threadStart || s.openRoot?.id === root || mentioned || held
  const next: AppState = {
    ...s,
    threads: keep ? { ...s.threads, [root]: upsertMessage(s.threads[root], m) } : s.threads,
    pending: nonce ? { ...s.pending, [c]: (s.pending[c] ?? []).filter((p) => p.nonce !== nonce) } : s.pending,
    typing: withoutTyping(s, c, m.author_id),
    // Replies are never unread; only a mention of me counts (until the thread is opened).
    reads: mentioned ? { ...s.reads, [c]: { ...read, mentions: read.mentions + 1 } } : s.reads,
  }
  if (known) return next
  return mapQuote(next, c, root, (q) => withThread(q, (t) => ({ ...t, reply_count: t.reply_count + 1, last_reply_id: maxIdOf(t.last_reply_id, m.id) })))
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
  const rootMsg = loadedRoot(s, c, root!)
  const rootThread = rootMsg ? threadOf(rootMsg) : null
  const read = s.reads[c]
  // Counted on creation only for a normal reply from someone else (see replyCreated).
  const kind = list?.find((x) => x.id === id)?.kind ?? 'normal'
  const mentioned = !!s.me && mentions.includes(s.me.id) && author_id !== s.me.id && kind === 'normal'
  // Only a mention not yet cleared by opening the thread is still in the count.
  // Where the user's seen point is: from Ready or a ThreadSeen, the loaded quote, or (a reply that
  // arrived live this session) nothing seen yet. Unknown: Ready lists every thread with a counted
  // mention, so this one had none and the deleted mention was not in the count.
  const known = s.threadSeen[root!] !== undefined || !!rootThread || !!list?.some((x) => x.id === id)
  const seen = [s.threadSeen[root!], rootThread?.replies_seen].reduce<string>((a, b) => (b && b > a ? b : a), '')
  const counted = mentioned && !!read && known && id > seen
  const next: AppState = {
    ...s,
    goneReplies: { ...s.goneReplies, [id]: { channel: c, root: root! } },
    threads: list ? { ...s.threads, [root!]: list.map((x) => (x.id === id ? { ...x, deleted: true, content: '' } : x)) } : s.threads,
    reads: counted ? { ...s.reads, [c]: { ...read, mentions: Math.max(0, read.mentions - 1) } } : s.reads,
  }
  return mapQuote(next, c, root!, (q) => withThread(q, (t) => ({ ...t, reply_count: Math.max(0, t.reply_count - 1) })))
}

function awardChanged(s: AppState, d: Extract<Event, { t: 'QuoteAwardChanged' }>['d']): AppState {
  const mine = (a: string[]) => !!s.me && a.includes(s.me.id)
  const f = (m: Message): Message =>
    m.quote ? { ...m, quote: { ...m.quote, lines: m.quote.lines.map((l) => (l.id === d.line_id ? { ...l, awards: d.count, awarders: d.awarders, awarded: mine(d.awarders) } : l)) } } : m
  return mapQotyMessages(mapQuote(s, d.channel_id, d.message_id, f), d.message_id, f)
}

function threadSeen(s: AppState, d: Extract<Event, { t: 'ThreadSeen' }>['d']): AppState {
  if (d.user_id !== s.me?.id) return s
  const read = s.reads[d.channel_id]
  const quoted = mapQuote(s, d.channel_id, d.root_id, (m) =>
    withThread(m, (t) => ({ ...t, replies_seen: maxIdOf(t.replies_seen, d.last_seen_id) })),
  )
  const seenNow = maxIdOf(s.threadSeen[d.root_id], d.last_seen_id)
  // A thread not open here holds only counted mentions: seen ones aren't counted any more.
  const kept = s.threads[d.root_id]
  const open = d.root_id in s.threadStart || s.openRoot?.id === d.root_id
  const left = kept && !open ? kept.filter((m) => m.id > seenNow) : kept
  const { [d.root_id]: _x, ...otherThreads } = s.threads
  const threads = left === kept ? quoted.threads : left!.length ? { ...s.threads, [d.root_id]: left! } : otherThreads
  const next = { ...quoted, threads, threadSeen: { ...s.threadSeen, [d.root_id]: seenNow } }
  if (!read) return next
  // The server's absolute count wins; an older server sends only the delta.
  const mentions = d.mentions ?? Math.max(0, read.mentions - d.mentions_cleared)
  return mentions === read.mentions ? next : { ...next, reads: { ...next.reads, [d.channel_id]: { ...read, mentions } } }
}

function dropFromFeeds(feeds: AppState['clipFeeds'], channel: string, id: string): AppState['clipFeeds'] {
  let next = feeds
  for (const [k, feed] of Object.entries(feeds)) {
    if (feed.channelId !== channel) continue
    const held = feed.items.some((m) => m.id === id)
    if (held || feed.loading) next = { ...next, [k]: { ...feed, items: feed.items.filter((m) => m.id !== id), gone: feed.loading ? [...(feed.gone ?? []), id] : feed.gone } }
  }
  return next
}

function withoutTyping(s: AppState, channel: string, user: string | null): AppState['typing'] {
  if (!user || !s.typing[channel]?.[user]) return s.typing
  const { [user]: _gone, ...rest } = s.typing[channel]
  return { ...s.typing, [channel]: rest }
}

/** Cache an opened profile. */
export function setProfile(s: AppState, p: Profile): AppState {
  return { ...s, profiles: { ...s.profiles, [p.user.id]: p } }
}

/** A changed profile replaces the user wherever we hold them; nobody new is added. */
function profileUpdated(s: AppState, d: Extract<Event, { t: 'ProfileUpdated' }>['d']): AppState {
  const { user, card } = d
  const cached = s.profiles[user.id]
  return {
    ...s,
    me: s.me?.id === user.id ? user : s.me,
    people: s.people[user.id] ? { ...s.people, [user.id]: { ...s.people[user.id], user } } : s.people,
    members: Object.fromEntries(
      Object.entries(s.members).map(([server, list]) => [server, list.some((m) => m.user.id === user.id) ? list.map((m) => (m.user.id === user.id ? { ...m, user } : m)) : list]),
    ),
    profiles: cached ? { ...s.profiles, [user.id]: { ...cached, user, card } } : s.profiles,
  }
}

export function applyEvent(s: AppState, e: Event, now: number): AppState {
  switch (e.t) {
    case 'MessageCreated': {
      if (e.d.message.thread_root_id) return replyCreated(s, e.d.message, e.d.nonce)
      const { message: m, nonce } = e.d
      const c = m.channel_id
      const feeds = m.clip ? clipCreated(s, m) : s
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
        // A clip joins the channel's feeds, not its text history.
        clipFeeds: feeds.clipFeeds,
        messages: m.clip || detached(s, c) ? s.messages : { ...s.messages, [c]: upsertMessage(s.messages[c], withMyQuoteState(m, undefined, s.me?.id)) },
        history: m.clip ? s.history : detached(s, c) ? { ...s.history, [c]: withMissed(s.history[c], m.id) } : s.history,
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
      // A QotY option's copy follows the edit too, loaded or not.
      const follow = (old: Message) => withMyQuoteState({ ...m, reactions: old.reactions }, old, s.me?.id)
      const q = mapOpenRoot(mapQotyMessages(s, m.id, follow), m.id, follow)
      if (m.clip) return clipUpdated(q, m)
      // History never loaded, or a detached window that doesn't hold it (adding it would leave a gap).
      if (!s.messages[c] || (detached(s, c) && !s.messages[c].some((x) => x.id === m.id))) return latest === s.latest ? q : { ...q, latest }
      return { ...q, latest, messages: { ...s.messages, [c]: upsertEdited(s.messages[c], withMyQuoteState(m, s.messages[c].find((x) => x.id === m.id), s.me?.id)) } }
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
      // Unsent replies to it can never post now (the thread is gone with it).
      const left = (s.pending[c] ?? []).filter((p) => p.thread_root_id !== id)
      const { [c]: _p, ...otherPending } = s.pending
      return {
        ...s,
        history,
        pending: left.length === (s.pending[c] ?? []).length ? s.pending : left.length ? { ...s.pending, [c]: left } : otherPending,
        openRoot: s.openRoot && s.openRoot.channel_id === c ? gone(s.openRoot) : s.openRoot,
        messages: s.messages[c] ? { ...s.messages, [c]: s.messages[c].map(gone) } : s.messages,
        clipFeeds: dropFromFeeds(s.clipFeeds, c, id),
        latest: s.latest[c] ? { ...s.latest, [c]: gone(s.latest[c]) } : s.latest,
        reads: wasUnread
          ? {
              ...s.reads,
              [c]: { ...read, unread: Math.max(0, read.unread - 1), mentions: Math.max(0, read.mentions - (mentioned ? 1 : 0)) },
            }
          : s.reads,
      }
    }
    case 'GameAdded':
      return { ...s, games: withGame(s.games, e.d.game) }
    case 'ClipViewChanged':
      return { ...s, clipViews: { ...s.clipViews, [e.d.channel_id]: e.d.view } }
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
    case 'ProfileUpdated':
      return profileUpdated(s, e.d)
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
        goneReplies: Object.fromEntries(Object.entries(s.goneReplies).filter(([, g]) => g.channel !== c)),
        openRoot: s.openRoot?.channel_id === c ? null : s.openRoot,
        channels: drop(s.channels),
        messages: drop(s.messages),
        dmMembers: drop(s.dmMembers),
        reads: drop(s.reads),
        latest: drop(s.latest),
        hidden: drop(s.hidden),
        pending: drop(s.pending),
        typing: drop(s.typing),
        history: drop(s.history),
        clipViews: drop(s.clipViews),
        clipFacets: drop(s.clipFacets),
        clipFeeds: Object.fromEntries(Object.entries(s.clipFeeds).filter(([, f]) => f.channelId !== c)),
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
      return { ...s, qoty: { ...s.qoty, [e.d.event.server_id]: withMyOptions(e.d.event, s.me?.id, s.qoty[e.d.event.server_id]) } }
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
      const base = mapFeedItems(s, c, id, apply) // a clip in the channel's feeds
      const loaded = !!findMessage(s, c, id) // not loaded: the fetch brings current reactions
      const inLatest = s.latest[c]?.id === id // the conversation preview keeps its own copy
      // a QotY option's copy and the open panel's quote, loaded or not (mapMessage reaches the latter when loaded)
      const q = loaded ? mapQotyMessages(base, id, apply) : mapOpenRoot(mapQotyMessages(base, id, apply), id, apply)
      if (!loaded && !inLatest) return q
      const next = loaded ? mapMessage(q, c, id, apply) : q
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
  if (!changed) return s
  // A reply to a quote known deleted can never post: it would sit failed forever.
  return Object.keys(saved).reduce((acc, channel) => withoutDeadReplies(acc, channel), { ...s, pending })
}

/** An unsent message as a normal one (yours), so it renders through the same path as a sent one. */
export function pendingMessage(p: Pending, channelId: string, meId: string): Message {
  return {
    id: `pending:${p.nonce}`, channel_id: channelId, author_id: meId, kind: 'normal', content: p.content,
    reply_to_id: p.reply_to_id, created_at: p.created_at, edited_at: null, deleted: false, mentions: [], reactions: [],
    thread_root_id: p.thread_root_id, quote: null, qoty_card: null, clip: null,
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
  const kept = list.slice(cut)
  const roots = new Set(kept.map((m) => m.id))
  if (s.openRoot) roots.add(s.openRoot.id)
  return {
    ...s,
    messages: { ...s.messages, [channel]: kept },
    history: h ? { ...s.history, [channel]: { ...h, start: false } } : s.history,
    // they only guard the reply counts of loaded quotes
    goneReplies: Object.fromEntries(Object.entries(s.goneReplies).filter(([, g]) => g.channel !== channel || roots.has(g.root))),
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
  return withoutDeadReplies({ ...s, messages: { ...s.messages, [channel]: list } }, channel)
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
  return withoutDeadReplies({ ...s, messages: { ...s.messages, [channel]: list }, history: { ...s.history, [channel]: h } }, channel)
}

/** Replace `channel`'s list with its latest page ("Jump to present"): attached again, unless a live
 * message newer than the page arrived meanwhile (then one `after` page fetches it). */
export function loadLatest(s: AppState, channel: string, page: Message[]): AppState {
  const list = sorted(page)
  const h = { ...reached(list, s.history[channel], true), start: list.length < PAGE }
  return withoutDeadReplies({ ...s, messages: { ...s.messages, [channel]: list }, history: { ...s.history, [channel]: h } }, channel)
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

/** The answer to my vote (`answer`, the event as the server saw it then) applied to the vote we
 * hold now: only my own vote moves, so newer votes from others (a QotyUpdated that overtook the
 * answer) stay. The gateway's QotyUpdated for my vote still follows with the absolute counts.
 * Ignored for another event, or when we already have it. */
export function applyMyVote(s: AppState, answer: QotyEvent | undefined): AppState {
  const meId = s.me?.id
  const cur = answer && s.qoty[answer.server_id]
  if (!answer || !cur || cur.id !== answer.id || !meId) return s
  const mineIn = (e: QotyEvent) => e.options.find((o) => o.voters.includes(meId))?.message.id ?? null
  const was = mineIn(cur)
  const now = mineIn(answer)
  if (was === now || (now !== null && !cur.options.some((o) => o.message.id === now))) return s
  const options = cur.options.map((o) => {
    const others = o.voters.filter((u) => u !== meId)
    const voters = o.message.id === now ? [...others, meId] : others
    return voters.length === o.voters.length && o.message.id !== now && o.message.id !== was ? o : { ...o, voters, votes: voters.length }
  })
  const voters = cur.voters + (was === null ? 1 : 0) - (now === null ? 1 : 0)
  return { ...s, qoty: { ...s.qoty, [cur.server_id]: { ...cur, options, voters } } }
}
