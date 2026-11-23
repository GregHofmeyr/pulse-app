// The open chat stays bounded while it stays open (#561): trimmed only while its view is at the latest.
import { describe, expect, it } from 'vitest'
import {
  addOlder, applyEvent, applyReady, canTrimShown, emptyState, mergeLatest, setShownAtLatest, setShownChannel, shownCut,
  trimHistory, SHOWN_KEEP, SHOWN_TRIM_AT, type AppState,
} from './state'
import type { Channel } from './protocol/Channel'
import type { Message } from './protocol/Message'
import type { Ready } from './protocol/Ready'

const ch = (id: string): Channel => ({ id, server_id: 'S1', kind: 'text', name: id, position: 0 })
const id = (n: number) => `M${String(n).padStart(5, '0')}`
const msg = (n: number, channel_id = 'C1'): Message => ({
  id: id(n), channel_id, author_id: 'U2', kind: 'normal', content: 'hi', reply_to_id: null,
  created_at: '2026-10-01T00:00:00.000Z', edited_at: null, deleted: false, mentions: [], reactions: [], thread_root_id: null, quote: null, qoty_card: null, clip: null, embeds: [],
})
const me = { id: 'U1', username: 'alex', display_name: 'alex', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
const ready: Ready = {
  me, servers: [{ id: 'S1', name: 'Main', icon_hash: null }], channels: [ch('C1'), ch('C2')],
  members: [{ server_id: 'S1', members: [{ user: me, nickname: null }] }], dm_members: [], voice: [], people: [], read_states: [], mutes: [],
  hidden: [], latest: [], streams: [], custom_emoji: [], frequent_emoji: [], sounds: [], sound_favourites: [], frequent_sounds: [],
  server_voice: [], status: { status: 'online', dnd_until: null }, appear_offline_to: [], live_notify: [], qoty: [], speaker_links: [],
  thread_seen: [], games: [], clip_views: [], private_nicknames: [],
}
const created = (n: number, channel = 'C1') => ({ t: 'MessageCreated', d: { message: msg(n, channel), nonce: null } }) as const
const readAt = (point: string | null, channel = 'C1') =>
  ({ t: 'ReadStateUpdated', d: { user_id: 'U1', channel_id: channel, last_read_message_id: point, mentions: null, unread: null } }) as const
const range = (from: number, to: number) => Array.from({ length: to - from + 1 }, (_, i) => msg(from + i))
const ids = (s: AppState, c = 'C1') => (s.messages[c] ?? []).map((m) => m.id)

/** C1 open with its latest page (messages 951..1000) loaded and read. */
function opened(): AppState {
  let s = setShownChannel(applyReady(emptyState(), ready), 'C1')
  s = mergeLatest(s, 'C1', range(951, 1000))
  return applyEvent(s, readAt(id(1000)), 0)
}
/** `count` live messages after `from`, each read on arrival (you're at the bottom, watching), checking `each` after every one. */
function post(s: AppState, from: number, count: number, each?: (s: AppState) => void): AppState {
  for (let n = from + 1; n <= from + count; n++) {
    s = applyEvent(s, created(n), 0)
    s = applyEvent(s, readAt(id(n)), 0)
    each?.(s)
  }
  return s
}

describe('canTrimShown', () => {
  const free = { atBottom: true, detached: false, replying: false, editingQuote: false, repliesOpen: false, jumping: false, flashing: false, backTo: false, holds: 0 }
  it('only at the latest with nothing held', () => {
    expect(canTrimShown(free)).toBe(true)
    for (const k of ['replying', 'editingQuote', 'repliesOpen', 'jumping', 'flashing', 'backTo', 'detached'] as const) {
      expect(canTrimShown({ ...free, [k]: true })).toBe(false)
    }
    expect(canTrimShown({ ...free, atBottom: false })).toBe(false)
    expect(canTrimShown({ ...free, holds: 1 })).toBe(false)
  })
})

describe('shownCut', () => {
  it('nothing until past SHOWN_TRIM_AT, then down to SHOWN_KEEP', () => {
    expect(shownCut(SHOWN_TRIM_AT, -1)).toBe(0)
    expect(shownCut(SHOWN_TRIM_AT + 1, -1)).toBe(SHOWN_TRIM_AT + 1 - SHOWN_KEEP)
    expect(shownCut(5000, -1)).toBe(5000 - SHOWN_KEEP)
  })
  it('keeps unread messages (the NEW line) unless they alone are past SHOWN_TRIM_AT', () => {
    expect(shownCut(700, 100)).toBe(100) // 600 unread kept
    expect(shownCut(700, 650)).toBe(300) // unread among the newest SHOWN_KEEP anyway
    expect(shownCut(700, 0)).toBe(300) // 700 unread: past the cap, so down to SHOWN_KEEP
  })
})

describe('the open chat while it stays open', () => {
  it('stays bounded while at the latest, through a long run of new messages', () => {
    let s = setShownAtLatest(opened(), 'C1', true)
    let trims = 0
    s = post(s, 1000, 5000, (t) => {
      const n = t.messages['C1'].length
      expect(n).toBeLessThanOrEqual(SHOWN_TRIM_AT)
      if (n === SHOWN_KEEP) trims++
    })
    expect(s.messages['C1'].length).toBeLessThanOrEqual(SHOWN_TRIM_AT)
    expect(s.messages['C1'].at(-1)!.id).toBe(id(6000))
    // trims are rare (hysteresis): about one per SHOWN_TRIM_AT - SHOWN_KEEP messages
    expect(trims).toBeLessThanOrEqual(Math.ceil(5050 / (SHOWN_TRIM_AT - SHOWN_KEEP)))
    // contiguous, newest kept, and older ones fetchable again
    const list = ids(s)
    expect(list).toEqual(range(6001 - list.length, 6000).map((m) => m.id))
    expect(s.history['C1'].start).toBe(false)
  })

  it('is never trimmed while scrolled up (or held), and trims once back at the latest', () => {
    let s = setShownAtLatest(opened(), 'C1', true)
    s = setShownAtLatest(s, 'C1', false) // scrolled up reading
    s = post(s, 1000, 2000)
    expect(s.messages['C1']).toHaveLength(2050)
    expect(s.messages['C1'][0].id).toBe(id(951)) // what you're reading is still there
    s = setShownAtLatest(s, 'C1', true) // back down
    expect(s.messages['C1']).toHaveLength(SHOWN_KEEP)
    expect(s.messages['C1'].at(-1)!.id).toBe(id(3000))
  })

  it('keeps everything without the view saying it is at the latest (unchanged before #561)', () => {
    const s = post(opened(), 1000, 1000)
    expect(s.messages['C1']).toHaveLength(1050)
  })

  it('keeps unread messages for the NEW line while you are away from the window', () => {
    let s = setShownAtLatest(opened(), 'C1', true)
    s = post(s, 1000, 100) // read, at 150
    for (let n = 1101; n <= 1600; n++) s = applyEvent(s, created(n), 0) // 500 unread, not watching
    expect(s.messages['C1'][0].id <= id(1101)).toBe(true)
    expect(ids(s)).toContain(id(1101))
    expect(s.messages['C1'].length).toBeLessThanOrEqual(SHOWN_TRIM_AT)
  })

  it('only the open chat, and the flag goes with it when another opens', () => {
    let s = setShownAtLatest(opened(), 'C2', true) // not the open one: ignored
    expect(s.trimmable).toBeUndefined()
    s = setShownAtLatest(s, 'C1', true)
    expect(s.trimmable).toBe('C1')
    s = setShownChannel(s, 'C2')
    expect(s.trimmable).toBeUndefined()
  })

  it('a detached window (a jump into the past) is never trimmed', () => {
    let s = setShownAtLatest(opened(), 'C1', true)
    s = { ...s, history: { ...s.history, C1: { ...s.history['C1'], end: false } } }
    s = post(s, 1000, 1000)
    // a detached list doesn't take live messages in at all, and what it holds is left as is
    expect(ids(s)).toEqual(range(951, 1000).map((m) => m.id))
  })

  it('scrolling up after a trim pages older history back in, with no gap or duplicate', () => {
    let s = setShownAtLatest(opened(), 'C1', true)
    s = post(s, 1000, 1000)
    const oldest = s.messages['C1'][0].id
    const n0 = Number(oldest.slice(1))
    s = setShownAtLatest(s, 'C1', false) // scrolled up to the top: the view asks for the page before its oldest
    const page = range(n0 - 50, n0 - 1).reverse() // newest first, as the server sends it
    s = addOlder(s, 'C1', oldest, page)
    const list = ids(s)
    expect(list).toEqual(range(n0 - 50, 2000).map((m) => m.id))
    expect(new Set(list).size).toBe(list.length)
    expect(s.history['C1'].start).toBe(false) // a full page: more above
    // a page fetched before a trim that lands after it is dropped (no hole), as for any trimmed chat
    const stale = addOlder(setShownAtLatest(post(setShownAtLatest(s, 'C1', true), 2000, 300), 'C1', true), 'C1', oldest, page)
    expect(ids(stale)[0]).not.toBe(id(n0 - 50))
  })

  it('leaving it still trims as before', () => {
    let s = setShownAtLatest(opened(), 'C1', true)
    s = post(s, 1000, 300)
    expect(trimHistory(setShownChannel(s, 'C2'), 'C1').messages['C1']).toHaveLength(100)
  })
})
