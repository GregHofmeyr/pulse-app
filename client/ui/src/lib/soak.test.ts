// Long-session soak and per-event cost of the reducer (state.ts). Pulse is left open for days: every structure the
// reducer (or a selector) fills has to stay bounded, and the work one gateway event costs mustn't grow with how much
// is held.
//
// What AppState holds, and what bounds it (checked by `checkBounds` below):
//   messages[c]         a chat that isn't open: `boundUnseen` (newest KEEP_ON_LEAVE plus unread, never past UNSEEN_CAP).
//                       The open chat (#561): while its view is at the latest, `boundShown` (past SHOWN_TRIM_AT, down
//                       to SHOWN_KEEP plus unread); while you're scrolled up (or something holds it), what it held then
//                       plus what is posted or paged in until you're back down. Leaving it trims it (`trimHistory`).
//   history, reads, typing, pending, hidden, latest, pins, clipViews, clipFacets, calls, voice, streams, stageShares,
//   watchParties, dmMembers   one entry per channel (dropped with the channel); `latest` per DM/group.
//   typing[c]           one entry per person typing in it; a Typing event drops the ones that ran out; Ready clears all.
//   pending[c]          your unsent messages; each confirmed one goes.
//   voice[c], streams[c]  one per person in the channel.
//   people, profiles    one per person you share something with (profiles: cards opened; cleared on Ready).
//   members[s]          the server's members; roleColors, serverVoice, layouts, games, qoty, speakerLinks: per server.
//   message.reactions   one per emoji on the message, each with at most every member in `users`.
//   threads             an open panel's thread, plus live mentions of you not yet seen (a ThreadSeen drops them);
//                       cleared on Ready. threadStart: open panels only. threadSeen: threads seen since Ready.
//   goneReplies         reply deletes since Ready (trimming a quote away drops its own).
//   clipFeeds           the feeds viewed since Ready (cleared on it): their pages plus clips posted since.
//   clipJobs            your own clips being sent.
//   customEmoji, sounds, frequent*, mutes, notifyLevels, ...  the servers' and your own settings: bounded by what
//                       exists, not by time.
// Caches outside AppState that the reducer or selectors fill: `mentionablesFor`'s memo (32 channels, selectors.ts),
// `serverNick`'s member index (a WeakMap on each member list, gone with it). markdown's render cache (2000) and
// linkCache (1000) are LRUs filled by the views, bounded in #544.
import { describe, expect, it } from 'vitest'
import {
  applyEvent, applyReady, emptyState, setShownChannel, trimHistory, addHistory, markHistory, mergeLatest, addOlder, addPending,
  setPoll, holdThread, addReplies, closeThread, openFeed, loadFeedPage, setShownAtLatest, UNSEEN_CAP, SHOWN_TRIM_AT, CLIP_PAGE, PAGE,
  type AppState,
} from './state'
import { feedKey } from './clips'
import type { Event } from './protocol/Event'
import type { Message } from './protocol/Message'
import type { Channel } from './protocol/Channel'
import type { Poll } from './protocol/Poll'
import type { Ready } from './protocol/Ready'
import type { User } from './protocol/User'
import { mentionablesFor, pillNamesFor, typingNames } from './selectors'

// --- fixtures ---------------------------------------------------------------------------------------------------

/** Sortable ids, like ULIDs: a bigger number is newer. */
let seq = 0
const nextId = () => String(++seq).padStart(26, '1')

const user = (id: string, name: string): User => ({ id, username: name, display_name: name, avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } })
const ME = user('U0', 'alex')
const NAMES = ['sam', 'kim', 'jo']
const people = (n: number): User[] => [ME, ...Array.from({ length: n - 1 }, (_, i) => user(`U${i + 1}`, `${NAMES[i % 3]}${i + 1}`))]

const message = (id: string, channel_id: string, author_id: string, mentions: string[] = []): Message => ({
  id, channel_id, author_id, kind: 'normal', content: `message ${id}`, reply_to_id: null,
  created_at: '2026-10-01T00:00:00.000Z', edited_at: null, deleted: false, mentions, reactions: [], thread_root_id: null, quote: null, qoty_card: null, clip: null, embeds: [],
})

const poll = (voters: number): Poll => ({ question: 'lunch?', options: ['yes', 'no'], multi: false, anonymous: false, ends_at: null, closed_at: null, closed: false, voters, mine: [], results: null })

type World = { servers: string[]; channels: Channel[]; voice: Channel[]; dms: Channel[]; users: User[] }

/** `servers` servers of `textPerServer` text channels (every fifth a quotes channel, every seventh a clips one) and a
 *  voice channel each, `dms` DMs and groups, and `members` people (you first), in every server. */
function world(servers: number, textPerServer: number, members: number, dms: number): World {
  const ids = Array.from({ length: servers }, (_, i) => `S${i}`)
  const channels: Channel[] = []
  const voice: Channel[] = []
  for (const s of ids) {
    for (let i = 0; i < textPerServer; i++) channels.push({ id: `${s}-T${i}`, server_id: s, kind: i % 7 === 6 ? 'clips' : i % 5 === 4 ? 'quotes' : 'text', name: `t${i}`, position: i })
    voice.push({ id: `${s}-V0`, server_id: s, kind: 'voice', name: 'Lounge', position: 0 })
  }
  const dmChannels: Channel[] = Array.from({ length: dms }, (_, i) => ({ id: `D${i}`, server_id: null, kind: i % 3 === 2 ? 'group' : 'dm', name: null, position: 0 }) as Channel)
  return { servers: ids, channels, voice, dms: dmChannels, users: people(members) }
}

function readyOf(w: World): Ready {
  const all = [...w.channels, ...w.voice, ...w.dms]
  return {
    me: ME,
    servers: w.servers.map((id) => ({ id, name: id, icon_hash: null }) as Ready['servers'][number]),
    channels: all,
    members: w.servers.map((server_id) => ({ server_id, members: w.users.map((u) => ({ user: u, nickname: null })) })),
    dm_members: w.dms.map((c, i) => ({ channel_id: c.id, user_ids: [ME.id, w.users[1 + (i % (w.users.length - 1))].id] })),
    voice: [],
    people: w.users.map((u) => ({ user: u, online: true, last_seen_at: null, status: 'online' as const })),
    read_states: [...w.channels, ...w.dms].map((c) => ({ channel_id: c.id, last_read_message_id: null, unread: 0, mentions: 0 })),
    mutes: [], hidden: [], latest: [], streams: [], custom_emoji: [], frequent_emoji: [], sounds: [], sound_favourites: [], frequent_sounds: [],
    server_voice: [], status: { status: 'online', dnd_until: null }, appear_offline_to: [], live_notify: [], qoty: [], speaker_links: [],
    thread_seen: [], games: [], clip_views: [], private_nicknames: [],
  } as Ready
}

/** A small seeded PRNG (mulberry32): the soak is the same run every time. */
function prng(seed: number): () => number {
  let a = seed
  return () => {
    a = (a + 0x6d2b79f5) | 0
    let t = Math.imul(a ^ (a >>> 15), 1 | a)
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296
  }
}

// --- soak -----------------------------------------------------------------------------------------------------------

describe('long-session soak', () => {
  it('a medium community over three days keeps every structure bounded', () => {
    const rand = prng(544)
    const pick = <T,>(xs: readonly T[]): T => xs[Math.floor(rand() * xs.length)]
    const chance = (p: number) => rand() < p

    const w = world(3, 15, 40, 10)
    const others = w.users.slice(1)
    const texts = w.channels.filter((c) => c.kind !== 'clips')
    const quotes = w.channels.filter((c) => c.kind === 'quotes')
    const clips = w.channels.filter((c) => c.kind === 'clips')
    const chats = [...texts, ...w.dms]
    const busy = [texts[0].id, texts[1].id] // two loud channels you rarely open
    const EMOJI = ['👍', '😂', '🔥', '❤️', '👀', '🎉']
    const CHANNELS = w.channels.length + w.voice.length + w.dms.length
    const MEMBERS = w.users.length

    // The server's side: every message ever posted, per channel (what pages are fetched from).
    const posted: Record<string, Message[]> = {}
    const replies: Record<string, Message[]> = {} // quote id -> its replies
    const post = (m: Message) => void (m.thread_root_id ? (replies[m.thread_root_id] ??= []) : (posted[m.channel_id] ??= [])).push(m)
    const recent = (c: string, n = 30) => (posted[c] ?? []).slice(-n)

    let now = 0
    let s = applyReady(emptyState(), readyOf(w))
    const apply = (e: Event) => {
      s = applyEvent(s, e, now)
    }

    // What has happened since the last Ready (the bounds of what Ready clears) and since the open chat was opened.
    let sinceReady = { mentionReplies: 0, replyDeletes: 0, threadsSeen: 0, feedsOpened: 0, clipsPosted: 0, feedPages: 0 }
    // ...and since you last scrolled up in it (while at the latest, it's trimmed instead)
    let shownSince = { posted: 0, paged: 0, held: 0 }
    let atLatest = true
    let openThread = null as { root: Message; channel: string } | null
    let maxHeld = 0
    let events = 0

    const open = (c: string) => {
      const left = s.shown
      s = setShownChannel(s, c)
      if (left && left !== c) s = trimHistory(s, left)
      const ch = s.channels[c]
      if (ch.kind === 'clips') {
        // the wall: the feed's newest page
        s = openFeed(s, c, {})
        s = loadFeedPage(s, feedKey(c, {}), (posted[c] ?? []).slice(-CLIP_PAGE).reverse())
        sinceReady.feedsOpened++
        sinceReady.feedPages++
      } else if (!s.history[c]?.loaded) {
        s = mergeLatest(s, c, recent(c, PAGE))
      }
      shownSince = { posted: 0, paged: 0, held: s.messages[c]?.length ?? 0 }
      // the view opens at the latest
      atLatest = true
      s = setShownAtLatest(s, c, true)
      const newest = (posted[c] ?? []).at(-1)
      if (newest && ch.kind !== 'clips') {
        apply({ t: 'ReadStateUpdated', d: { user_id: ME.id, channel_id: c, last_read_message_id: newest.id, unread: 0, mentions: 0 } })
      }
    }

    const create = (c: string, author: string) => {
      const ch = s.channels[c]
      const m = message(nextId(), c, author, chance(0.05) ? [ME.id] : [])
      if (ch.kind === 'quotes') m.quote = { lines: [], year: 2026, said_on: null, reply_count: 0, last_reply_id: null, replies_seen: null, wins: [] }
      if (ch.kind === 'clips') {
        m.clip = { thumb_hash: '', duration_ms: 1000, width: 1, height: 1, size: 1, game_id: null, reply_count: 0, last_reply_id: null, replies_seen: null }
        sinceReady.clipsPosted++
      }
      if (chance(0.02)) m.poll = poll(0)
      let nonce: string | null = null
      if (author === ME.id) {
        nonce = `n${m.id}`
        s = addPending(s, c, { nonce, content: m.content, reply_to_id: null, thread_root_id: null, status: 'pending', created_at: m.created_at })
      }
      post(m)
      if (c === s.shown) shownSince.posted++
      apply({ t: 'MessageCreated', d: { message: m, nonce } })
    }

    const reply = (c: string) => {
      const root = pick(recent(c, 20))
      if (!root) return
      const mentioned = chance(0.1)
      const m = { ...message(nextId(), c, pick(others).id, mentioned ? [ME.id] : []), thread_root_id: root.id }
      if (mentioned) sinceReady.mentionReplies++
      post(m)
      apply({ t: 'MessageCreated', d: { message: m, nonce: null } })
    }

    /** One simulated minute of a busy evening, or a quiet night. */
    const minute = (hour: number) => {
      const quiet = hour % 24 < 7
      const n = quiet ? 2 : 12
      for (let i = 0; i < n; i++) {
        events++
        const r = rand()
        const c = chance(0.3) ? pick(busy) : pick(chats).id
        if (r < 0.4) create(c, chance(0.05) ? ME.id : pick(others).id)
        else if (r < 0.45) create(pick(clips).id, pick(others).id)
        else if (r < 0.5) reply(pick(quotes).id)
        else if (r < 0.55) {
          const m = pick(recent(c, 20))
          if (m) apply({ t: 'MessageUpdated', d: { message: { ...m, content: `${m.content} (edited)`, edited_at: '2026-10-01T00:01:00.000Z' } } })
        } else if (r < 0.58) {
          const m = pick(recent(c, 20))
          if (m) apply({ t: 'MessageDeleted', d: { channel_id: c, message_id: m.id, author_id: m.author_id, mentions: m.mentions, thread_root_id: null } })
        } else if (r < 0.6) {
          const q = pick(quotes).id
          const root = pick(Object.keys(replies).filter((id) => posted[q]?.some((m) => m.id === id)))
          const gone = root && pick(replies[root])
          if (gone) {
            sinceReady.replyDeletes++
            apply({ t: 'MessageDeleted', d: { channel_id: q, message_id: gone.id, author_id: gone.author_id, mentions: gone.mentions, thread_root_id: root } })
          }
        } else if (r < 0.7) {
          const m = pick(recent(c, 20))
          const u = pick(others).id
          if (m) apply({ t: chance(0.7) ? 'ReactionAdded' : 'ReactionRemoved', d: { channel_id: c, message_id: m.id, emoji: pick(EMOJI), user_id: u, count: 1 + Math.floor(rand() * MEMBERS) } })
        } else if (r < 0.8) apply({ t: 'Typing', d: { channel_id: c, user_id: pick(others).id } })
        else if (r < 0.86) apply({ t: 'PresenceChanged', d: { user_id: pick(others).id, online: chance(0.8), last_seen_at: null, status: pick(['online', 'dnd', 'offline'] as const), idle: chance(0.2) } })
        else if (r < 0.88) apply({ t: 'PlayingChanged', d: { user_id: pick(others).id, playing: chance(0.5) ? { name: 'Deadlock', steam_app_id: null, started_at: null, art: null } : null } })
        else if (r < 0.9) apply({ t: 'ListeningChanged', d: { user_id: pick(others).id, listening: chance(0.5) ? { title: 'song', artist: null, album: null, spotify_id: null, art: null } : null } })
        else if (r < 0.94) {
          const v = pick(w.voice).id
          const u = pick(others).id
          const inIt = s.voice[v]?.some((x) => x.user_id === u)
          apply(inIt ? { t: 'VoiceLeft', d: { channel_id: v, user_id: u } } : { t: 'VoiceJoined', d: { channel_id: v, user_id: u, flags: { muted: chance(0.3), deafened: false } } })
          if (inIt) apply({ t: 'StreamEnded', d: { channel_id: v, user_id: u, reason: 'stopped' } } as Event)
          else if (chance(0.2)) apply({ t: 'StreamStarted', d: { stream: { channel_id: v, user_id: u, settings: { height: 720, fps: 30, audio: true, allow_ghosts: false }, status: 'live', viewers: [], ghosts: 0, started_at: '' } } })
        } else if (r < 0.95) {
          const m = recent(c, 50).findLast((x) => x.poll)
          if (m) s = setPoll(s, c, m.id, poll(1 + Math.floor(rand() * MEMBERS)))
        } else if (r < 0.96) {
          const d = pick(w.dms).id
          apply({ t: 'ConversationVisibility', d: { channel_id: d, hidden: chance(0.5) } } as Event)
        } else if (r < 0.97 && s.shown && s.history[s.shown]?.loaded) {
          const shown = s.shown
          if (!atLatest && chance(0.3)) {
            // back down to the latest in the open chat (#561): it's trimmed if it grew past SHOWN_TRIM_AT
            atLatest = true
            s = setShownAtLatest(s, shown, true)
          } else {
            // scrolling up in the open chat
            if (atLatest) {
              atLatest = false
              s = setShownAtLatest(s, shown, false)
              shownSince = { posted: 0, paged: 0, held: s.messages[shown]?.length ?? 0 }
            }
            const held = s.messages[shown] ?? []
            const all = posted[shown] ?? []
            const at = held.length ? all.findIndex((m) => m.id === held[0].id) : -1
            if (at > 0) {
              const page = all.slice(Math.max(0, at - PAGE), at)
              s = addOlder(s, shown, held[0].id, page)
              shownSince.paged += page.length
            }
          }
        } else if (r < 0.975) {
          // a quote's replies panel, opened and later closed
          if (openThread) {
            s = closeThread(s, openThread.root.id, openThread.channel)
            openThread = null
          } else {
            const q = pick(quotes).id
            const root = s.messages[q]?.at(-1)
            if (root) {
              openThread = { root, channel: q }
              s = addReplies(holdThread(s, root), root.id, replies[root.id] ?? [], true)
            }
          }
        } else if (r < 0.98) {
          const root = pick(Object.keys(s.threads))
          const q = root && s.threads[root][0]?.channel_id
          if (q) {
            sinceReady.threadsSeen++
            apply({ t: 'ThreadSeen', d: { user_id: ME.id, channel_id: q, root_id: root, last_seen_id: s.threads[root].at(-1)!.id, mentions_cleared: 0, mentions: 0 } })
          }
        }
      }
      now += 60_000
    }

    const checkBounds = (when: string) => {
      const keyed = ['messages', 'history', 'reads', 'typing', 'pending', 'hidden', 'latest', 'pins', 'clipViews', 'clipFacets', 'calls', 'voice', 'streams', 'stageShares', 'watchParties', 'dmMembers'] as const
      for (const k of keyed) expect(Object.keys(s[k]).length, `${when}: ${k}`).toBeLessThanOrEqual(CHANNELS)
      expect(Object.keys(s.latest).length, `${when}: latest`).toBeLessThanOrEqual(w.dms.length)
      let held = 0
      for (const [c, list] of Object.entries(s.messages)) {
        held += list.length
        const cap = c !== s.shown ? UNSEEN_CAP : atLatest ? SHOWN_TRIM_AT : shownSince.held + shownSince.posted + shownSince.paged
        expect(list.length, `${when}: messages in ${c}`).toBeLessThanOrEqual(cap)
        for (const m of list) {
          expect(m.reactions.length).toBeLessThanOrEqual(EMOJI.length)
          for (const x of m.reactions) expect((x.users ?? []).length).toBeLessThanOrEqual(MEMBERS)
        }
      }
      maxHeld = Math.max(maxHeld, held)
      for (const t of Object.values(s.typing)) expect(Object.keys(t).length, `${when}: typing`).toBeLessThanOrEqual(MEMBERS)
      for (const v of Object.values(s.voice)) expect(v.length, `${when}: voice`).toBeLessThanOrEqual(MEMBERS)
      for (const v of Object.values(s.streams)) expect(Object.keys(v).length, `${when}: streams`).toBeLessThanOrEqual(MEMBERS)
      for (const list of Object.values(s.members)) expect(list.length).toBeLessThanOrEqual(MEMBERS)
      expect(Object.keys(s.people).length, `${when}: people`).toBeLessThanOrEqual(MEMBERS)
      expect(Object.keys(s.profiles).length, `${when}: profiles`).toBeLessThanOrEqual(MEMBERS)
      // your sends are all confirmed by the end of each minute
      expect(Object.values(s.pending).flat(), `${when}: pending`).toHaveLength(0)
      const openReplies = openThread ? (replies[openThread.root.id]?.length ?? 0) : 0
      expect(Object.keys(s.threads).length, `${when}: threads`).toBeLessThanOrEqual(sinceReady.mentionReplies + 1)
      expect(Object.values(s.threads).flat().length, `${when}: thread replies`).toBeLessThanOrEqual(sinceReady.mentionReplies + openReplies)
      expect(Object.keys(s.threadStart).length, `${when}: threadStart`).toBeLessThanOrEqual(1)
      expect(Object.keys(s.threadSeen).length, `${when}: threadSeen`).toBeLessThanOrEqual(sinceReady.threadsSeen)
      expect(Object.keys(s.goneReplies).length, `${when}: goneReplies`).toBeLessThanOrEqual(sinceReady.replyDeletes)
      expect(Object.keys(s.clipFeeds).length, `${when}: clipFeeds`).toBeLessThanOrEqual(Math.min(sinceReady.feedsOpened, clips.length))
      for (const f of Object.values(s.clipFeeds)) expect(f.items.length, `${when}: feed`).toBeLessThanOrEqual(sinceReady.feedPages * CLIP_PAGE + sinceReady.clipsPosted)
      expect(Object.keys(s.clipJobs)).toHaveLength(0)
      // the selectors the views read every event
      expect(mentionablesFor(s, chats[2].id).length).toBeLessThanOrEqual(MEMBERS)
      expect(pillNamesFor(s, chats[2].id).length).toBeLessThanOrEqual(MEMBERS)
      expect(typingNames(s, null, w.dms[0].id, now).length).toBeLessThanOrEqual(MEMBERS)
    }

    const DAYS = 3
    const heldByDay: number[] = []
    for (let hour = 0; hour < DAYS * 24; hour++) {
      // a reconnect every few hours: a fresh snapshot
      if (hour > 0 && hour % 5 === 0) {
        if (openThread) s = closeThread(s, openThread.root.id, openThread.channel)
        openThread = null
        s = applyReady(s, readyOf(w))
        sinceReady = { mentionReplies: 0, replyDeletes: 0, threadsSeen: 0, feedsOpened: 0, clipsPosted: 0, feedPages: 0 }
        // the open chat reloads its latest page
        if (s.shown) open(s.shown)
      }
      // you move around a few times an hour, rarely to the busy channels
      for (let m = 0; m < 60; m++) {
        if (m % 15 === 0 && hour % 24 >= 7) open(chance(0.1) ? pick(busy) : chance(0.15) ? pick(clips).id : pick(chats.filter((c) => !busy.includes(c.id))).id)
        minute(hour)
      }
      checkBounds(`day ${Math.floor(hour / 24) + 1} ${hour % 24}:00`)
      if (hour % 24 === 23) heldByDay.push(maxHeld)
    }

    expect(events).toBeGreaterThan(20_000)
    // The busy channels went far past what a chat that isn't open keeps.
    for (const c of busy) expect(posted[c].length).toBeGreaterThan(2 * UNSEEN_CAP)
    // The busy channels hold no more than a chat that isn't open keeps, however much was posted in them.
    for (const c of busy) if (c !== s.shown || atLatest) expect(s.messages[c].length).toBeLessThanOrEqual(UNSEEN_CAP)
    const total = Object.values(posted).flat().length
    console.log(`soak: ${events} events, ${total} messages posted, most held at once by day: ${heldByDay.join(', ')}`)
    expect(Math.max(...heldByDay)).toBeLessThanOrEqual(CHANNELS * UNSEEN_CAP)
  })
})

describe('the open chat left open for days (#561)', () => {
  it('stays bounded while at the latest, grows only while scrolled up, and pages back in without gaps', () => {
    const rand = prng(561)
    const chance = (p: number) => rand() < p
    const w = world(1, 2, 10, 0)
    const others = w.users.slice(1)
    const c = w.channels[0].id
    const posted: Message[] = []
    let s = setShownChannel(applyReady(emptyState(), readyOf(w)), c)
    let atLatest = true
    let focused = true
    let since = { held: 0, posted: 0, paged: 0 }
    let trims = 0
    const read = () => {
      const newest = posted.at(-1)
      if (newest) s = applyEvent(s, { t: 'ReadStateUpdated', d: { user_id: ME.id, channel_id: c, last_read_message_id: newest.id, unread: 0, mentions: 0 } }, 0)
    }
    const load = () => {
      s = mergeLatest(s, c, posted.slice(-PAGE))
      atLatest = true
      s = setShownAtLatest(s, c, true)
    }
    load()
    for (let hour = 0; hour < 72; hour++) {
      if (hour > 0 && hour % 5 === 0) {
        s = applyReady(s, readyOf(w))
        load()
      }
      focused = hour % 24 >= 8 // away from the window overnight: unread piles up
      for (let m = 0; m < 60; m++) {
        // a busy channel: a few messages a minute
        for (let i = 0; i < 1 + Math.floor(rand() * 4); i++) {
          const msg = message(nextId(), c, others[Math.floor(rand() * others.length)].id)
          posted.push(msg)
          const before = s.messages[c]?.length ?? 0
          s = applyEvent(s, { t: 'MessageCreated', d: { message: msg, nonce: null } }, 0)
          if ((s.messages[c]?.length ?? 0) < before) trims++
          if (!atLatest) since.posted++
          if (focused && atLatest) read()
        }
        // now and then you scroll up through history, page by page, and later come back down
        if (focused && atLatest && chance(0.01)) {
          atLatest = false
          s = setShownAtLatest(s, c, false)
          since = { held: s.messages[c].length, posted: 0, paged: 0 }
        } else if (!atLatest && chance(0.3)) {
          const held = s.messages[c]
          const at = posted.findIndex((x) => x.id === held[0].id)
          if (at > 0) {
            const page = posted.slice(Math.max(0, at - PAGE), at).reverse()
            s = addOlder(s, c, held[0].id, page)
            since.paged += page.length
          }
          if (chance(0.15)) {
            atLatest = true
            s = setShownAtLatest(s, c, true)
            if (focused) read()
          }
        }
      }
      const list = s.messages[c]
      expect(list.length, `hour ${hour}`).toBeLessThanOrEqual(atLatest ? SHOWN_TRIM_AT : since.held + since.posted + since.paged)
      // what's held is one run of the channel's messages, up to the newest: no gap, no duplicate
      const from = posted.findIndex((x) => x.id === list[0].id)
      expect(list.map((x) => x.id), `hour ${hour}`).toEqual(posted.slice(from).map((x) => x.id))
    }
    expect(posted.length).toBeGreaterThan(10 * SHOWN_TRIM_AT)
    expect(trims).toBeGreaterThan(10)
  })
})

// --- per-event cost -----------------------------------------------------------------------------------------------

describe('reducer cost per event, large state', () => {
  // 50 channels × 500 messages, 200 people.
  const w = world(5, 10, 200, 10)
  let base = applyReady(emptyState(), readyOf(w))
  for (const c of w.channels) {
    const page = Array.from({ length: 500 }, () => message(nextId(), c.id, w.users[1 + (seq % 199)].id))
    base = markHistory(addHistory(base, c.id, page), c.id, false)
  }
  const shown = w.channels[0].id
  base = setShownChannel(base, shown)

  /** Mean µs per event over `n` events, after a warm-up. Each event is applied to the state the last one left. */
  function time(n: number, make: (i: number) => Event, after: (s: AppState) => unknown = () => {}): number {
    let s = base
    for (let i = 0; i < 200; i++) after((s = applyEvent(s, make(i), 1_000_000 + i)))
    const t0 = performance.now()
    for (let i = 0; i < n; i++) after((s = applyEvent(s, make(i), 1_000_000 + i)))
    return ((performance.now() - t0) / n) * 1000
  }

  it('reports µs per event (and stays well under a frame)', () => {
    const author = (i: number) => w.users[1 + (i % 199)].id
    const presence = (i: number): Event => ({ t: 'PresenceChanged', d: { user_id: author(i), online: i % 2 === 0, last_seen_at: null, status: i % 2 === 0 ? 'online' : 'offline' } })
    const results: Record<string, number> = {
      'MessageCreated (open chat)': time(2000, (i) => ({ t: 'MessageCreated', d: { message: message(nextId(), shown, author(i)), nonce: null } })),
      'MessageCreated (other chat)': time(2000, (i) => ({ t: 'MessageCreated', d: { message: message(nextId(), w.channels[1 + (i % 49)].id, author(i)), nonce: null } })),
      PresenceChanged: time(5000, presence),
      'PresenceChanged + mentionablesFor': time(2000, presence, (s) => mentionablesFor(s, shown)),
      Typing: time(5000, (i) => ({ t: 'Typing', d: { channel_id: w.channels[i % 50].id, user_id: author(i) } })),
      ReactionAdded: time(5000, (i) => {
        const list = base.messages[shown]
        return { t: 'ReactionAdded', d: { channel_id: shown, message_id: list[list.length - 1 - (i % 50)].id, emoji: '👍', user_id: author(i), count: 1 + (i % 7) } }
      }),
    }
    const table = Object.entries(results).map(([k, v]) => `  ${k.padEnd(34)} ${v.toFixed(1).padStart(8)} µs`).join('\n')
    console.log(`reducer cost per event (50 channels × 500 messages, 200 people):\n${table}`)
    // Generous: a frame is 16 ms, and CI runners are slow and shared. This only catches a blow-up.
    for (const v of Object.values(results)) expect(v).toBeLessThan(2000)
  })
})
