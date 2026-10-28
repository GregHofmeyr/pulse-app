import { describe, expect, it } from 'vitest'
import { applyEvent, applyReady, applyMyVote, emptyState, addPending, pendingMessage, removePending, restoreUnsent, markHistory, addHistory, addReplies, holdThread, openFeed, loadFeedPage, setClipJob, dropClipJob, setProfile, type AppState, type Pending } from './state'
import { feedKey, clipView, type ClipFilter } from './clips'
import type { Ready } from './protocol/Ready'
import type { Message } from './protocol/Message'
import type { Event } from './protocol/Event'
import type { QotyEvent } from './protocol/QotyEvent'
import type { Channel } from './protocol/Channel'
import type { Quote } from './protocol/Quote'
import type { StreamInfo } from './protocol/StreamInfo'

const ch = (id: string, kind: Channel['kind'] = 'text', server_id: string | null = 'S1', position = 0): Channel => ({
  id, server_id, kind, name: id, position,
})
const msg = (id: string, channel_id = 'C1', content = 'hi', author_id: string | null = 'U2'): Message => ({
  id, channel_id, author_id, kind: 'normal', content, reply_to_id: null,
  created_at: '2026-10-01T00:00:00.000Z', edited_at: null, deleted: false, mentions: [], reactions: [], thread_root_id: null, quote: null, qoty_card: null, clip: null,
})
const me = { id: 'U1', username: 'alex', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
const ready = (over: Partial<Ready> = {}): Ready => ({
  me,
  servers: [{ id: 'S1', name: 'Main', icon_hash: null }],
  channels: [ch('C1'), ch('V1', 'voice')],
  members: [{ server_id: 'S1', members: [{ user: me, nickname: null }] }],
  dm_members: [],
  voice: [{ channel_id: 'V1', members: [{ user_id: 'U2', flags: { muted: false, deafened: false } }] }],
  people: [],
  read_states: [],
  mutes: [],
  hidden: [],
  latest: [],
  streams: [],
  custom_emoji: [],
  frequent_emoji: [], qoty: [], speaker_links: [], thread_seen: [], games: [], clip_views: [],
  ...over,
})

describe('applyReady', () => {
  it('populates everything', () => {
    const s = applyReady(emptyState(), ready())
    expect(s.me?.id).toBe('U1')
    expect(s.servers).toHaveLength(1)
    expect(Object.keys(s.channels)).toEqual(['C1', 'V1'])
    expect(s.members['S1']).toHaveLength(1)
    expect(s.voice['V1'][0].user_id).toBe('U2')
  })

  it('keeps loaded history for channels that still exist on reconnect', () => {
    let s = applyReady(emptyState(), ready())
    s = applyEvent(s, { t: 'MessageCreated', d: { message: msg('M1'), nonce: null } }, 0)
    s = applyReady(s, ready({ channels: [ch('C1')] }))
    expect(s.messages['C1'].map((m) => m.id)).toEqual(['M1'])
    expect(s.voice['V1']).toBeDefined()
  })

  it('drops history for channels that disappeared', () => {
    let s = applyReady(emptyState(), ready())
    s = applyEvent(s, { t: 'MessageCreated', d: { message: msg('M1'), nonce: null } }, 0)
    s = applyReady(s, ready({ channels: [ch('V1', 'voice')] }))
    expect(s.messages['C1']).toBeUndefined()
  })
})

describe('applyEvent', () => {
  const base = () => applyReady(emptyState(), ready())

  it('appends messages and dedupes by id', () => {
    let s = base()
    const e = { t: 'MessageCreated', d: { message: msg('M1'), nonce: null } } as const
    s = applyEvent(applyEvent(s, e, 0), e, 0)
    expect(s.messages['C1']).toHaveLength(1)
  })

  it('replaces a pending optimistic message with the same nonce', () => {
    let s = addPending(base(), 'C1', { nonce: 'n1', content: 'hi', reply_to_id: null, thread_root_id: null, status: 'pending', created_at: '2026-10-01T00:00:00.000Z' })
    expect(s.pending['C1']).toHaveLength(1)
    s = applyEvent(s, { t: 'MessageCreated', d: { message: msg('M1', 'C1', 'hi', 'U1'), nonce: 'n1' } }, 0)
    expect(s.pending['C1']).toHaveLength(0)
    expect(s.messages['C1']).toHaveLength(1)
  })

  it('updates and deletes in place', () => {
    let s = applyEvent(base(), { t: 'MessageCreated', d: { message: msg('M1'), nonce: null } }, 0)
    s = applyEvent(s, { t: 'MessageUpdated', d: { message: { ...msg('M1'), content: 'edited', edited_at: 'x' } } }, 0)
    expect(s.messages['C1'][0].content).toBe('edited')
    s = applyEvent(s, { t: 'MessageDeleted', d: { channel_id: 'C1', message_id: 'M1', author_id: 'U2', mentions: [], thread_root_id: null } }, 0)
    expect(s.messages['C1'][0].deleted).toBe(true)
    expect(s.messages['C1'][0].content).toBe('')
  })

  it('tracks voice joins, leaves and flags', () => {
    let s = base()
    const off = { muted: false, deafened: false }
    s = applyEvent(s, { t: 'VoiceJoined', d: { channel_id: 'V1', user_id: 'U1', flags: off } }, 0)
    s = applyEvent(s, { t: 'VoiceJoined', d: { channel_id: 'V1', user_id: 'U1', flags: off } }, 0)
    expect(s.voice['V1']).toHaveLength(2)
    s = applyEvent(s, { t: 'VoiceStateChanged', d: { channel_id: 'V1', user_id: 'U1', flags: { muted: true, deafened: false } } }, 0)
    expect(s.voice['V1'].find((m) => m.user_id === 'U1')?.flags.muted).toBe(true)
    s = applyEvent(s, { t: 'VoiceLeft', d: { channel_id: 'V1', user_id: 'U2' } }, 0)
    expect(s.voice['V1'].map((m) => m.user_id)).toEqual(['U1'])
    s = applyEvent(s, { t: 'VoiceLeft', d: { channel_id: 'V1', user_id: 'U1' } }, 0)
    expect(s.voice['V1']).toBeUndefined()
  })

  it('joining voice muted shows as muted (flags travel with the join)', () => {
    const s = applyEvent(base(), { t: 'VoiceJoined', d: { channel_id: 'V9', user_id: 'U1', flags: { muted: true, deafened: false } } }, 0)
    expect(s.voice['V9'][0].flags.muted).toBe(true)
  })

  it('typing expires after 6 s and clears when that user posts', () => {
    let s = applyEvent(base(), { t: 'Typing', d: { channel_id: 'C1', user_id: 'U2' } }, 1000)
    expect(s.typing['C1']['U2']).toBe(7000)
    s = applyEvent(s, { t: 'MessageCreated', d: { message: msg('M1', 'C1', 'hi', 'U2'), nonce: null } }, 2000)
    expect(s.typing['C1']['U2']).toBeUndefined()
  })

  it('member joins and channel creation are idempotent', () => {
    let s = base()
    const joined = { t: 'MemberJoined', d: { server_id: 'S1', member: { user: { id: 'U2', username: 'sam', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] as string[] } }, nickname: null } } } as const
    s = applyEvent(applyEvent(s, joined, 0), joined, 0)
    expect(s.members['S1']).toHaveLength(2)
    const created = { t: 'ChannelCreated', d: { channel: ch('C2') } } as const
    s = applyEvent(applyEvent(s, created, 0), created, 0)
    expect(Object.keys(s.channels)).toContain('C2')
  })

  it('server created adds a tab once', () => {
    const e = { t: 'ServerCreated', d: { server: { id: 'S2', name: 'Other', icon_hash: null } } } as const
    const s = applyEvent(applyEvent(base(), e, 0), e, 0)
    expect(s.servers.map((x) => x.id)).toEqual(['S1', 'S2'])
  })
})

describe('history markers (I2)', () => {
  it('start empty, are set on load, and a new Ready clears them so the open channel refetches', async () => {
    const { markHistory } = await import('./state')
    let s = applyReady(emptyState(), ready())
    expect(s.history['C1']).toBeUndefined()
    s = markHistory(s, 'C1', true)
    expect(s.history['C1']).toEqual({ loaded: true, start: true, end: true })
    s = applyReady(s, ready())
    expect(s.history['C1']).toBeUndefined()
    expect(emptyState().history).toEqual({})
  })
})

describe('deleting an unread message', () => {
  const dmReady = () =>
    applyReady(emptyState(), ready({
      channels: [ch('C1'), ch('D1', 'dm', null)],
      dm_members: [{ channel_id: 'D1', user_ids: ['U1', 'U2'] }],
      read_states: [{ channel_id: 'D1', last_read_message_id: 'M1', unread: 2, mentions: 1 }],
    }))
  const del = (id: string, author: string | null, mentions: string[] = []) =>
    ({ t: 'MessageDeleted', d: { channel_id: 'D1', message_id: id, author_id: author, mentions, thread_root_id: null } }) as const

  it('takes it back out of the unread and mention counts', () => {
    let s = applyEvent(dmReady(), del('M3', 'U2', ['U1']), 0)
    expect(s.reads['D1']).toMatchObject({ unread: 1, mentions: 0 })
    s = applyEvent(s, del('M2', 'U2'), 0)
    expect(s.reads['D1']).toMatchObject({ unread: 0, mentions: 0 })
    s = applyEvent(s, del('M4', 'U2', ['U1']), 0) // counts never go negative
    expect(s.reads['D1']).toMatchObject({ unread: 0, mentions: 0 })
  })

  it('leaves the counts alone for read messages and your own', () => {
    expect(applyEvent(dmReady(), del('M0', 'U2', ['U1']), 0).reads['D1']).toMatchObject({ unread: 2, mentions: 1 })
    expect(applyEvent(dmReady(), del('M5', 'U1'), 0).reads['D1']).toMatchObject({ unread: 2, mentions: 1 })
  })

  it('updates the conversation list preview', () => {
    let s = applyEvent(dmReady(), { t: 'MessageCreated', d: { message: msg('M9', 'D1', 'secret'), nonce: null } }, 0)
    s = applyEvent(s, del('M9', 'U2'), 0)
    expect(s.latest['D1']).toMatchObject({ id: 'M9', deleted: true, content: '' })
  })
})

it('an edit to the latest message updates the preview', () => {
  let s = applyReady(emptyState(), ready({ channels: [ch('D1', 'dm', null)], dm_members: [{ channel_id: 'D1', user_ids: ['U1', 'U2'] }] }))
  s = applyEvent(s, { t: 'MessageCreated', d: { message: msg('M1', 'D1', 'typo'), nonce: null } }, 0)
  s = { ...s, messages: {} } // history not loaded (e.g. never opened)
  s = applyEvent(s, { t: 'MessageUpdated', d: { message: { ...msg('M1', 'D1', 'fixed'), edited_at: 'x' } } }, 0)
  expect(s.latest['D1'].content).toBe('fixed')
})

it('an edit to the latest message keeps the preview\'s reactions (counts and me)', () => {
  let s = applyReady(emptyState(), ready({ channels: [ch('D1', 'dm', null)], dm_members: [{ channel_id: 'D1', user_ids: ['U1', 'U2'] }] }))
  const m = { ...msg('M1', 'D1', 'typo'), reactions: [{ emoji: '👍', count: 2, me: true }] }
  s = applyEvent(s, { t: 'MessageCreated', d: { message: m, nonce: null } }, 0)
  // The server sends me:false for everyone on edits.
  const edited = { ...msg('M1', 'D1', 'fixed'), edited_at: 'x', reactions: [{ emoji: '👍', count: 2, me: false }] }
  for (const loaded of [true, false]) {
    const s1 = applyEvent(loaded ? s : { ...s, messages: {} }, { t: 'MessageUpdated', d: { message: edited } }, 0)
    expect(s1.latest['D1'].content).toBe('fixed')
    expect(s1.latest['D1'].reactions).toEqual([{ emoji: '👍', count: 2, me: true }])
  }
})

it('removal from a conversation forgets everything about it, unsent messages included', () => {
  let s = applyReady(emptyState(), ready({ channels: [ch('G1', 'group', null)], dm_members: [{ channel_id: 'G1', user_ids: ['U1', 'U2', 'U3'] }] }))
  s = addPending(s, 'G1', { nonce: 'n1', content: 'are you there?', reply_to_id: null, thread_root_id: null, status: 'failed', created_at: '2026-10-01T00:00:00.000Z' })
  s = applyEvent(s, { t: 'Typing', d: { channel_id: 'G1', user_id: 'U2' } }, 0)
  s = applyEvent(s, { t: 'ChannelRemoved', d: { channel_id: 'G1', user_id: 'U1' } }, 0)
  expect(s.pending['G1']).toBeUndefined()
  expect(s.typing['G1']).toBeUndefined()
  expect(s.history['G1']).toBeUndefined()
})

describe('trimming history on leave', () => {
  const page = (from: number, n: number) =>
    Array.from({ length: n }, (_, i) => msg(`M${String(from + i).padStart(4, '0')}`))

  it('keeps the newest KEEP_ON_LEAVE messages and makes older ones fetchable again', async () => {
    const { addHistory, markHistory, trimHistory, KEEP_ON_LEAVE } = await import('./state')
    let s = applyReady(emptyState(), ready())
    s = markHistory(addHistory(s, 'C1', page(0, 300)), 'C1', true)
    const t = trimHistory(s, 'C1')
    expect(t.messages['C1']).toHaveLength(KEEP_ON_LEAVE)
    expect(t.messages['C1'][0].id).toBe('M0200')
    expect(t.messages['C1'].at(-1)!.id).toBe('M0299')
    expect(t.history['C1']).toEqual({ loaded: true, start: false, end: true })
  })

  it('leaves short channels (and the state object) alone', async () => {
    const { addHistory, trimHistory } = await import('./state')
    const s = addHistory(applyReady(emptyState(), ready()), 'C1', page(0, 40))
    expect(trimHistory(s, 'C1')).toBe(s)
    expect(trimHistory(s, 'nope')).toBe(s)
  })

  it('an older page that lands after a trim is dropped instead of leaving a hole', async () => {
    const { addHistory, addOlder, trimHistory } = await import('./state')
    let s = addHistory(applyReady(emptyState(), ready()), 'C1', page(100, 200)) // M0100..M0299
    const before = s.messages['C1'][0].id // scroll-up asks for messages before M0100
    s = trimHistory(s, 'C1') // you switched away meanwhile: M0200..M0299 remain
    const late = addOlder(s, 'C1', before, page(50, 50)) // M0050..M0099 arrives
    expect(late).toBe(s)
    const fresh = addOlder(s, 'C1', 'M0200', page(150, 50)) // a page for the current head is added
    expect(fresh.messages['C1'][0].id).toBe('M0150')
    expect(fresh.messages['C1']).toHaveLength(150)
  })

  it('a SHORT late page after a trim does not mark the trimmed list as reaching the start', async () => {
    const { addHistory, addOlder, markHistory, trimHistory } = await import('./state')
    let s = markHistory(addHistory(applyReady(emptyState(), ready()), 'C1', page(100, 200)), 'C1', false)
    const before = s.messages['C1'][0].id
    s = trimHistory(s, 'C1')
    const late = addOlder(s, 'C1', before, page(90, 10)) // short page: would mean "start reached"
    expect(late).toBe(s)
    expect(late.history['C1'].start).toBe(false)
    const fresh = addOlder(s, 'C1', 'M0200', page(190, 10)) // a genuine short page does mark the start
    expect(fresh.history['C1'].start).toBe(true)
  })
})

describe('trimming keeps unread messages', () => {
  const page = (from: number, n: number) =>
    Array.from({ length: n }, (_, i) => msg(`M${String(from + i).padStart(4, '0')}`))
  const readAt = (point: string | null) =>
    ({ t: 'ReadStateUpdated', d: { user_id: 'U1', channel_id: 'C1', last_read_message_id: point, mentions: null, unread: null } }) as const

  it('never trims past your read point, so unread counts survive a trim', async () => {
    const { addHistory, trimHistory } = await import('./state')
    let s = addHistory(applyReady(emptyState(), ready()), 'C1', page(0, 300)) // M0000..M0299
    s = applyEvent(s, readAt('M0049'), 0) // 250 unread
    expect(s.reads['C1'].unread).toBe(250)
    const t = trimHistory(s, 'C1')
    expect(t.messages['C1'][0].id).toBe('M0050') // the oldest unread is kept
    expect(applyEvent(t, readAt('M0049'), 0).reads['C1'].unread).toBe(250)
  })

  it('a channel you have never read is not trimmed at all', async () => {
    const { addHistory, trimHistory } = await import('./state')
    let s = addHistory(applyReady(emptyState(), ready()), 'C1', page(0, 300))
    s = applyEvent(s, readAt(null), 0)
    expect(trimHistory(s, 'C1')).toBe(s)
  })

  it('a read channel still trims to KEEP_ON_LEAVE', async () => {
    const { addHistory, trimHistory, KEEP_ON_LEAVE } = await import('./state')
    let s = addHistory(applyReady(emptyState(), ready()), 'C1', page(0, 300))
    s = applyEvent(s, readAt('M0299'), 0)
    expect(trimHistory(s, 'C1').messages['C1']).toHaveLength(KEEP_ON_LEAVE)
  })
})

describe('streams', () => {
  const info = (user_id: string, over: Partial<StreamInfo> = {}, channel_id = 'V1'): StreamInfo => ({
    channel_id, user_id,
    settings: { height: 720, fps: 60, audio: true },
    status: 'live', viewers: [], started_at: '2026-10-23T10:00:00.000Z',
    ...over,
  })
  const base = () => applyReady(emptyState(), ready())

  it('ready_fills_streams', () => {
    const s = applyReady(emptyState(), ready({ streams: [info('U2'), info('U3'), info('U4', {}, 'V2')] }))
    expect(Object.keys(s.streams['V1']).sort()).toEqual(['U2', 'U3'])
    expect(s.streams['V2']['U4'].user_id).toBe('U4')
    // A reconnect replaces, not merges: a stream missing from the new Ready is gone.
    expect(applyReady(s, ready()).streams).toEqual({})
  })

  it('stream_started_adds_and_updated_replaces', () => {
    let s = applyEvent(base(), { t: 'StreamStarted', d: { stream: info('U2') } }, 0)
    expect(s.streams['V1']['U2'].status).toBe('live')
    s = applyEvent(s, { t: 'StreamUpdated', d: { stream: info('U2', { status: 'reconnecting', viewers: ['U1'] }) } }, 0)
    expect(s.streams['V1']['U2'].status).toBe('reconnecting')
    expect(s.streams['V1']['U2'].viewers).toEqual(['U1'])
    expect(Object.keys(s.streams['V1'])).toEqual(['U2'])
  })

  it('stream_ended_removes', () => {
    let s = applyEvent(base(), { t: 'StreamStarted', d: { stream: info('U2') } }, 0)
    s = applyEvent(s, { t: 'StreamStarted', d: { stream: info('U3') } }, 0)
    s = applyEvent(s, { t: 'StreamEnded', d: { channel_id: 'V1', user_id: 'U2', reason: 'stopped' } }, 0)
    expect(Object.keys(s.streams['V1'])).toEqual(['U3'])
    s = applyEvent(s, { t: 'StreamEnded', d: { channel_id: 'V1', user_id: 'U3', reason: 'timed_out' } }, 0)
    expect(s.streams['V1']).toBeUndefined()
    // Ending something we never saw changes nothing.
    expect(applyEvent(s, { t: 'StreamEnded', d: { channel_id: 'V1', user_id: 'U9', reason: 'stopped' } }, 0)).toEqual(s)
  })

  it('viewers_changed_updates_viewers_only', () => {
    let s = applyEvent(base(), { t: 'StreamStarted', d: { stream: info('U2', { status: 'reconnecting' }) } }, 0)
    s = applyEvent(s, { t: 'ViewersChanged', d: { channel_id: 'V1', user_id: 'U2', viewers: ['U1', 'U3'] } }, 0)
    expect(s.streams['V1']['U2']).toEqual(info('U2', { status: 'reconnecting', viewers: ['U1', 'U3'] }))
    // Viewers of an unknown stream are ignored rather than inventing one.
    const same = applyEvent(s, { t: 'ViewersChanged', d: { channel_id: 'V1', user_id: 'U9', viewers: ['U1'] } }, 0)
    expect(same.streams).toEqual(s.streams)
  })

  it('voice_left_does_not_touch_streams', () => {
    let s = applyEvent(base(), { t: 'StreamStarted', d: { stream: info('U2') } }, 0)
    s = applyEvent(s, { t: 'VoiceLeft', d: { channel_id: 'V1', user_id: 'U2' } }, 0)
    expect(s.streams['V1']['U2']).toBeDefined() // the server follows up with StreamEnded
  })
})

describe('reactions and custom emoji', () => {
  const withMsg = (reactions: Message['reactions'] = []) => {
    let s = applyReady(emptyState(), ready())
    return applyEvent(s, { t: 'MessageCreated', d: { message: { ...msg('M1'), reactions }, nonce: null } }, 0)
  }
  const react = (t: 'ReactionAdded' | 'ReactionRemoved', user_id: string, count: number, emoji = '👍') =>
    ({ t, d: { channel_id: 'C1', message_id: 'M1', emoji, user_id, count } }) as const

  it('reaction events keep a conversation\'s latest preview current, loaded or not', () => {
    let s = applyReady(emptyState(), ready({ channels: [ch('D1', 'dm', null)], dm_members: [{ channel_id: 'D1', user_ids: ['U1', 'U2'] }] }))
    s = applyEvent(s, { t: 'MessageCreated', d: { message: msg('M1', 'D1'), nonce: null } }, 0)
    const ev = (t: 'ReactionAdded' | 'ReactionRemoved', user_id: string, count: number) =>
      ({ t, d: { channel_id: 'D1', message_id: 'M1', emoji: '👍', user_id, count } }) as const
    for (const loaded of [true, false]) {
      let s1 = loaded ? s : { ...s, messages: {} }
      s1 = applyEvent(s1, ev('ReactionAdded', 'U1', 1), 0)
      s1 = applyEvent(s1, ev('ReactionAdded', 'U2', 2), 0)
      expect(s1.latest['D1'].reactions, String(loaded)).toEqual([{ emoji: '👍', count: 2, me: true }])
      if (loaded) expect(s1.messages['D1'][0].reactions).toEqual([{ emoji: '👍', count: 2, me: true }])
      s1 = applyEvent(s1, ev('ReactionRemoved', 'U1', 1), 0)
      expect(s1.latest['D1'].reactions).toEqual([{ emoji: '👍', count: 1, me: false }])
    }
  })

  it('counts another user\'s reaction without touching me', () => {
    const s = applyEvent(withMsg(), react('ReactionAdded', 'U2', 1), 0)
    expect(s.messages['C1'][0].reactions).toEqual([{ emoji: '👍', count: 1, me: false }])
  })

  it('sets me when the reaction is mine, and clears it on removal', () => {
    let s = applyEvent(withMsg(), react('ReactionAdded', 'U1', 2), 0)
    expect(s.messages['C1'][0].reactions).toEqual([{ emoji: '👍', count: 2, me: true }])
    s = applyEvent(s, react('ReactionRemoved', 'U2', 1), 0)
    expect(s.messages['C1'][0].reactions[0]).toEqual({ emoji: '👍', count: 1, me: true })
    s = applyEvent(s, react('ReactionRemoved', 'U1', 0), 0)
    expect(s.messages['C1'][0].reactions).toEqual([])
  })

  it('appends new emoji after existing ones', () => {
    let s = applyEvent(withMsg(), react('ReactionAdded', 'U2', 1), 0)
    s = applyEvent(s, react('ReactionAdded', 'U2', 1, '🎉'), 0)
    expect(s.messages['C1'][0].reactions.map((r) => r.emoji)).toEqual(['👍', '🎉'])
  })

  it('lets the absolute count win over event order', () => {
    let s = applyEvent(withMsg(), react('ReactionAdded', 'U2', 2), 0)
    s = applyEvent(s, react('ReactionRemoved', 'U3', 1), 0)
    expect(s.messages['C1'][0].reactions[0].count).toBe(1)
  })

  it('ignores reactions on messages that are not loaded', () => {
    const s = applyReady(emptyState(), ready())
    expect(applyEvent(s, react('ReactionAdded', 'U2', 1), 0)).toBe(s)
  })

  it('keeps reactions (counts and me) when a message is edited', () => {
    const s0 = withMsg([{ emoji: '👍', count: 3, me: true }])
    // The server sends me:false for everyone on edits.
    const edited = { ...msg('M1', 'C1', 'edited'), edited_at: '2026-10-01T00:01:00.000Z', reactions: [{ emoji: '👍', count: 1, me: false }] }
    const s = applyEvent(s0, { t: 'MessageUpdated', d: { message: edited } }, 0)
    expect(s.messages['C1'][0].content).toBe('edited')
    expect(s.messages['C1'][0].reactions).toEqual([{ emoji: '👍', count: 3, me: true }])
  })

  it('loads custom and frequent emoji from Ready and upserts Emoji events', () => {
    const pog = { id: 'E1', server_id: 'S1', name: 'pog', hash: 'h', still_hash: null, animated: false, added_by: 'U1', created_at: '2026-10-01T00:00:00.000Z' }
    let s = applyReady(emptyState(), ready({ custom_emoji: [pog], frequent_emoji: ['👍'] }))
    expect(s.customEmoji['E1'].name).toBe('pog')
    expect(s.frequentEmoji).toEqual(['👍'])
    s = applyEvent(s, { t: 'EmojiUpdated', d: { emoji: { ...pog, name: 'pogger' } } }, 0)
    expect(s.customEmoji['E1'].name).toBe('pogger')
    s = applyEvent(s, { t: 'EmojiDeleted', d: { emoji: { ...pog, name: 'pogger', hash: null } } }, 0)
    expect(s.customEmoji['E1']).toMatchObject({ name: 'pogger', hash: null })
    s = applyEvent(s, { t: 'EmojiCreated', d: { emoji: { ...pog, id: 'E2', name: 'new' } } }, 0)
    expect(Object.keys(s.customEmoji)).toEqual(['E1', 'E2'])
  })

  it('replaces the frequent list on FrequentEmoji', () => {
    const s0 = applyReady(emptyState(), ready({ frequent_emoji: ['👍', '😂'] }))
    const s = applyEvent(s0, { t: 'FrequentEmoji', d: { user_id: 'U1', frequent_emoji: ['🎉', '👍'] } }, 0)
    expect(s.frequentEmoji).toEqual(['🎉', '👍'])
  })
})

describe('unsent messages', () => {
  const unsent = (nonce: string, status: Pending['status'] = 'pending'): Pending => ({ nonce, content: `text ${nonce}`, reply_to_id: null, thread_root_id: null, status, created_at: '2026-10-01T00:00:00.000Z' })

  it('restore_marks_everything_failed', () => {
    const s = restoreUnsent(applyReady(emptyState(), ready()), { C1: [unsent('a'), unsent('b', 'pending')] })
    expect(s.pending['C1'].map((p) => p.status)).toEqual(['failed', 'failed'])
  })

  it('restore_drops_gone_channels_and_known_nonces', () => {
    let s = applyReady(emptyState(), ready())
    s = addPending(s, 'C1', unsent('a'))
    s = restoreUnsent(s, { C1: [unsent('a'), unsent('b')], GONE: [unsent('c')] })
    expect(s.pending['C1'].map((p) => p.nonce)).toEqual(['a', 'b'])
    expect(s.pending['GONE']).toBeUndefined()
  })

  it('remove_pending_drops_one', () => {
    let s = applyReady(emptyState(), ready())
    s = addPending(addPending(s, 'C1', unsent('a')), 'C1', unsent('b'))
    expect(removePending(s, 'C1', 'a').pending['C1'].map((p) => p.nonce)).toEqual(['b'])
    expect(removePending(removePending(s, 'C1', 'a'), 'C1', 'b').pending['C1']).toBeUndefined()
  })

  it('pending_message_renders_as_you', () => {
    const m = pendingMessage(unsent('a'), 'C1', 'U1')
    expect(m).toMatchObject({ author_id: 'U1', channel_id: 'C1', kind: 'normal', content: 'text a', deleted: false, created_at: '2026-10-01T00:00:00.000Z' })
  })
})


describe('quotes', () => {
  const sam = { id: 'U2', username: 'sam', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
  const line = (id: string, over: Partial<Quote['lines'][number]> = {}): Quote['lines'][number] => ({
    id, speaker_id: null, speaker_name: 'Sam', text: 'hi', media: null, caption: null, awards: 0, awarders: [], awarded: false, ...over,
  })
  const quote = (over: Partial<Quote> = {}): Quote => ({ lines: [line('L1')], year: 2026, said_on: null, reply_count: 0, last_reply_id: null, replies_seen: null, wins: [], ...over })
  const qmsg = (id: string, q: Partial<Quote> = {}): Message => ({ ...msg(id, 'Q1', ''), quote: quote(q) })
  const reply = (id: string, root: string, over: Partial<Message> = {}): Message => ({ ...msg(id, 'Q1', 'ok'), thread_root_id: root, ...over })
  const qready = () => applyReady(emptyState(), ready({
    channels: [ch('Q1', 'quotes')],
    members: [{ server_id: 'S1', members: [{ user: me, nickname: null }, { user: sam, nickname: null }] }],
    read_states: [{ channel_id: 'Q1', last_read_message_id: null, unread: 0, mentions: 0 }],
  }))
  const created = (s: AppState, m: Message, nonce: string | null = null) => applyEvent(s, { t: 'MessageCreated', d: { message: m, nonce } }, 0)
  const withQuote = (q: Partial<Quote> = {}) => created(qready(), qmsg('M1', q))
  const quoteOf = (s: AppState, id = 'M1') => s.messages['Q1'].find((m) => m.id === id)!.quote!
  /** M1's thread as an open panel holds it: fetched (live replies join only a fetched thread). */
  const fetched = (s: AppState) => addReplies(s, 'M1', [], true)

  it('a reply goes to its thread, not the list', () => {
    const s = created(fetched(withQuote()), reply('R1', 'M1'))
    expect(s.messages['Q1'].map((m) => m.id)).toEqual(['M1'])
    expect(s.threads['M1'].map((m) => m.id)).toEqual(['R1'])
    expect(quoteOf(s)).toMatchObject({ reply_count: 1, last_reply_id: 'R1' })
  })

  it('a reply seen twice (outbox and gateway) counts once and replaces its pending', () => {
    let s = addPending(fetched(withQuote()), 'Q1', { nonce: 'n', content: 'ok', reply_to_id: null, thread_root_id: 'M1', status: 'pending', created_at: '2026-10-01T00:00:00.000Z' })
    s = created(s, reply('R1', 'M1'), 'n')
    s = created(s, reply('R1', 'M1'))
    expect(s.pending['Q1']).toEqual([])
    expect(s.threads['M1']).toHaveLength(1)
    expect(quoteOf(s).reply_count).toBe(1)
  })

  it('replies keep thread order and the newest reply id', () => {
    let s = created(fetched(withQuote()), reply('R2', 'M1'))
    s = created(s, reply('R1', 'M1'))
    expect(s.threads['M1'].map((m) => m.id)).toEqual(['R1', 'R2'])
    expect(quoteOf(s).last_reply_id).toBe('R2')
  })

  it('a reply is never unread but its mention counts', () => {
    let s = created(withQuote(), reply('R1', 'M1', { author_id: 'U2' }))
    expect(s.reads['Q1']).toMatchObject({ unread: 1, mentions: 0 }) // the quote itself, not its reply
    s = created(s, reply('R2', 'M1', { author_id: 'U2', mentions: ['U1'] }))
    expect(s.reads['Q1']).toMatchObject({ unread: 1, mentions: 1 })
    s = created(s, reply('R3', 'M1', { author_id: 'U1', mentions: ['U1'] }))
    expect(s.reads['Q1'].mentions).toBe(1) // your own reply never counts
    s = created(s, reply('R2', 'M1', { author_id: 'U2', mentions: ['U1'] }))
    expect(s.reads['Q1'].mentions).toBe(1) // the same reply again
  })

  it('a mention reply counts even when a page already brought its quote with that reply as the newest (fix-wave re-review)', () => {
    // sam's R1 mentions me; a history page brings M1 (last_reply_id R1) before R1's frame arrives.
    let s = addHistory(qready(), 'Q1', [qmsg('M1', { reply_count: 1, last_reply_id: 'R1' })])
    s = created(s, reply('R1', 'M1', { author_id: 'U2', mentions: ['U1'] }))
    expect(s.reads['Q1'].mentions).toBe(1)
    expect(quoteOf(s)).toMatchObject({ reply_count: 1, last_reply_id: 'R1' }) // already counted by the page
    s = created(s, reply('R1', 'M1', { author_id: 'U2', mentions: ['U1'] }))
    expect(s.reads['Q1'].mentions).toBe(1) // a repeat still counts once
    // A page whose quote I've already seen past the reply (opened elsewhere): not counted.
    s = addHistory(qready(), 'Q1', [qmsg('M1', { reply_count: 1, last_reply_id: 'R1', replies_seen: 'R1' })])
    s = created(s, reply('R1', 'M1', { author_id: 'U2', mentions: ['U1'] }))
    expect(s.reads['Q1'].mentions).toBe(0)
  })

  it('a quote in a quotes channel is unread; a winner card is not', () => {
    let s = created(qready(), qmsg('M1'))
    expect(s.reads['Q1'].unread).toBe(1)
    s = created(s, { ...msg('M2', 'Q1', ''), kind: 'system', author_id: null })
    expect(s.reads['Q1'].unread).toBe(1)
  })

  it('a quotes channel gets a read point of its own when none was sent', () => {
    const s = created(applyReady(emptyState(), ready({ channels: [ch('Q1', 'quotes')] })), qmsg('M1'))
    expect(s.reads['Q1'].unread).toBe(1)
  })

  it('ThreadSeen clears the reply mentions', () => {
    let s = created(withQuote(), reply('R1', 'M1', { author_id: 'U2', mentions: ['U1'] }))
    s = created(s, reply('R2', 'M1', { author_id: 'U2', mentions: ['U1'] }))
    s = applyEvent(s, { t: 'ThreadSeen', d: { user_id: 'U1', channel_id: 'Q1', root_id: 'M1', last_seen_id: 'R2', mentions_cleared: 2, mentions: null } }, 0)
    expect(s.reads['Q1'].mentions).toBe(0)
    expect(quoteOf(s).replies_seen).toBe('R2')
    // floors at zero, and the seen point never moves back
    s = applyEvent(s, { t: 'ThreadSeen', d: { user_id: 'U1', channel_id: 'Q1', root_id: 'M1', last_seen_id: 'R1', mentions_cleared: 3, mentions: null } }, 0)
    expect(s.reads['Q1'].mentions).toBe(0)
    expect(quoteOf(s).replies_seen).toBe('R2')
  })

  it('ThreadSeen without a cleared mention only moves the seen point', () => {
    let s = created(withQuote(), reply('R1', 'M1', { author_id: 'U2', mentions: ['U1'] }))
    s = applyEvent(s, { t: 'ThreadSeen', d: { user_id: 'U1', channel_id: 'Q1', root_id: 'M1', last_seen_id: 'R1', mentions_cleared: 0, mentions: null } }, 0)
    expect(s.reads['Q1'].mentions).toBe(1)
    expect(quoteOf(s).replies_seen).toBe('R1')
  })

  it('a read-state update takes the server\'s absolute mention count, replies included', () => {
    // A quotes channel read elsewhere, never loaded here: two reply mentions stay counted.
    let s = applyReady(emptyState(), ready({ channels: [ch('Q1', 'quotes')], read_states: [{ channel_id: 'Q1', last_read_message_id: null, unread: 3, mentions: 5 }] }))
    s = applyEvent(s, { t: 'ReadStateUpdated', d: { user_id: 'U1', channel_id: 'Q1', last_read_message_id: 'M9', mentions: 2, unread: null } }, 0)
    expect(s.reads['Q1']).toMatchObject({ last_read_message_id: 'M9', unread: 0, mentions: 2 })
    // A text channel read elsewhere and never opened here: everything is read.
    s = applyReady(emptyState(), ready({ read_states: [{ channel_id: 'C1', last_read_message_id: null, unread: 4, mentions: 4 }] }))
    s = applyEvent(s, { t: 'ReadStateUpdated', d: { user_id: 'U1', channel_id: 'C1', last_read_message_id: 'M9', mentions: 0, unread: null } }, 0)
    expect(s.reads['C1']).toMatchObject({ unread: 0, mentions: 0 })
  })

  it('an older server\'s read-state update (no count) recounts what is loaded', () => {
    let s = applyReady(emptyState(), ready({ read_states: [{ channel_id: 'C1', last_read_message_id: null, unread: 0, mentions: 0 }] }))
    s = applyEvent(s, { t: 'MessageCreated', d: { message: { ...msg('M1'), mentions: ['U1'] }, nonce: null } }, 0)
    s = applyEvent(s, { t: 'MessageCreated', d: { message: { ...msg('M2'), mentions: ['U1'] }, nonce: null } }, 0)
    s = applyEvent(s, { t: 'ReadStateUpdated', d: { user_id: 'U1', channel_id: 'C1', last_read_message_id: 'M1', mentions: null, unread: null } }, 0)
    expect(s.reads['C1']).toMatchObject({ unread: 1, mentions: 1 })
  })

  it('ThreadSeen takes the absolute count when it has one', () => {
    let s = created(withQuote(), reply('R1', 'M1', { author_id: 'U2', mentions: ['U1'] }))
    s = applyEvent(s, { t: 'ThreadSeen', d: { user_id: 'U1', channel_id: 'Q1', root_id: 'M1', last_seen_id: 'R1', mentions_cleared: 1, mentions: 4 } }, 0)
    expect(s.reads['Q1'].mentions).toBe(4)
  })

  it('the seen point is kept even when the quote is not loaded, and a later delete uses it', () => {
    let s = qready()
    s = applyEvent(s, { t: 'ThreadSeen', d: { user_id: 'U1', channel_id: 'Q1', root_id: 'M1', last_seen_id: 'R2', mentions_cleared: 0, mentions: null } }, 0)
    expect(s.threadSeen['M1']).toBe('R2')
    s = { ...s, reads: { ...s.reads, Q1: { ...s.reads['Q1'], mentions: 1 } } }
    const del = (id: string): Event => ({ t: 'MessageDeleted', d: { channel_id: 'Q1', message_id: id, author_id: 'U2', mentions: ['U1'], thread_root_id: 'M1' } })
    expect(applyEvent(s, del('R1'), 0).reads['Q1'].mentions).toBe(1) // already seen: not in the count
    expect(applyEvent(s, del('R3'), 0).reads['Q1'].mentions).toBe(0) // unseen: leaves it
  })

  it('Ready seeds the thread seen points, and a delete of an unknown one does not lower the count', () => {
    let s = applyReady(emptyState(), ready({
      channels: [ch('Q1', 'quotes')],
      read_states: [{ channel_id: 'Q1', last_read_message_id: null, unread: 0, mentions: 2 }],
      thread_seen: [{ root_id: 'M1', last_seen_id: 'R1' }, { root_id: 'M2', last_seen_id: null }],
    }))
    expect(s.threadSeen).toEqual({ M1: 'R1', M2: '' })
    const del = (root: string, id: string): Event => ({ t: 'MessageDeleted', d: { channel_id: 'Q1', message_id: id, author_id: 'U2', mentions: ['U1'], thread_root_id: root } })
    // M3 is not listed: no counted mention there, so a deleted mention was not in the count.
    expect(applyEvent(s, del('M3', 'R5'), 0).reads['Q1'].mentions).toBe(2)
    // M2 was never seen: any reply is counted.
    expect(applyEvent(s, del('M2', 'R5'), 0).reads['Q1'].mentions).toBe(1)
    // M1 seen up to R1: R1 is not counted, R2 is.
    expect(applyEvent(s, del('M1', 'R1'), 0).reads['Q1'].mentions).toBe(2)
    expect(applyEvent(s, del('M1', 'R2'), 0).reads['Q1'].mentions).toBe(1)
    // Another Ready resets them.
    s = applyReady(s, ready({ channels: [ch('Q1', 'quotes')] }))
    expect(s.threadSeen).toEqual({})
    expect(s.goneReplies).toEqual({})
  })

  it('a live mention in a thread whose seen point is unknown is counted, and so is its delete', () => {
    let s = created(qready(), reply('R1', 'M1', { author_id: 'U2', mentions: ['U1'] }))
    expect(s.reads['Q1'].mentions).toBe(1)
    s = applyEvent(s, { t: 'MessageDeleted', d: { channel_id: 'Q1', message_id: 'R1', author_id: 'U2', mentions: ['U1'], thread_root_id: 'M1' } }, 0)
    expect(s.reads['Q1'].mentions).toBe(0)
  })

  it('a system reply\'s mention was never counted, so deleting it lowers nothing', () => {
    let s = created(qready(), reply('R1', 'M1', { author_id: 'U2', kind: 'system', mentions: ['U1'] }))
    expect(s.reads['Q1'].mentions).toBe(0)
    s = { ...s, reads: { ...s.reads, Q1: { ...s.reads['Q1'], mentions: 1 } } } // another counted mention
    s = applyEvent(s, { t: 'MessageDeleted', d: { channel_id: 'Q1', message_id: 'R1', author_id: 'U2', mentions: ['U1'], thread_root_id: 'M1' } }, 0)
    expect(s.reads['Q1'].mentions).toBe(1)
  })

  it('removing a channel prunes its gone replies too', () => {
    let s = created(withQuote(), reply('R1', 'M1'))
    s = applyEvent(s, { t: 'MessageDeleted', d: { channel_id: 'Q1', message_id: 'R1', author_id: 'U2', mentions: [], thread_root_id: 'M1' } }, 0)
    expect(s.goneReplies['R1']).toBeDefined()
    s = applyEvent(s, { t: 'ChannelRemoved', d: { channel_id: 'Q1', user_id: 'U1' } }, 0)
    expect(s.goneReplies).toEqual({})
  })

  it('a delete seen twice for a reply that is not loaded lowers the count once', () => {
    let s = withQuote({ reply_count: 2, last_reply_id: 'R2' })
    const del: Event = { t: 'MessageDeleted', d: { channel_id: 'Q1', message_id: 'R2', author_id: 'U2', mentions: [], thread_root_id: 'M1' } }
    s = applyEvent(s, del, 0)
    s = applyEvent(s, del, 0)
    expect(quoteOf(s).reply_count).toBe(1)
  })

  it('an edited reply updates its thread, not the list', () => {
    let s = created(fetched(withQuote()), reply('R1', 'M1'))
    s = applyEvent(s, { t: 'MessageUpdated', d: { message: reply('R1', 'M1', { content: 'fixed', edited_at: '2026-10-01T01:00:00.000Z' }) } }, 0)
    expect(s.threads['M1'][0].content).toBe('fixed')
    expect(s.messages['Q1'].map((m) => m.id)).toEqual(['M1'])
  })

  it('deleting a reply lowers reply_count', () => {
    let s = created(fetched(withQuote()), reply('R1', 'M1'))
    s = created(s, reply('R2', 'M1'))
    const del = (id: string, mentions: string[] = []): Event => ({ t: 'MessageDeleted', d: { channel_id: 'Q1', message_id: id, author_id: 'U2', mentions, thread_root_id: 'M1' } })
    s = applyEvent(s, del('R2'), 0)
    expect(quoteOf(s).reply_count).toBe(1)
    expect(s.threads['M1'].find((m) => m.id === 'R2')).toMatchObject({ deleted: true, content: '' })
    expect(s.messages['Q1'].map((m) => m.id)).toEqual(['M1'])
    s = applyEvent(s, del('R2'), 0) // again: no second decrement
    expect(quoteOf(s).reply_count).toBe(1)
    s = applyEvent(s, del('R1'), 0)
    s = applyEvent(s, del('R1'), 0)
    expect(quoteOf(s).reply_count).toBe(0)
  })

  it('deleting a reply that mentioned me takes it out of the mention count unless seen', () => {
    let s = created(withQuote(), reply('R1', 'M1', { author_id: 'U2', mentions: ['U1'] }))
    s = applyEvent(s, { t: 'MessageDeleted', d: { channel_id: 'Q1', message_id: 'R1', author_id: 'U2', mentions: ['U1'], thread_root_id: 'M1' } }, 0)
    expect(s.reads['Q1'].mentions).toBe(0)
    s = created(s, reply('R2', 'M1', { author_id: 'U2', mentions: ['U1'] }))
    s = applyEvent(s, { t: 'ThreadSeen', d: { user_id: 'U1', channel_id: 'Q1', root_id: 'M1', last_seen_id: 'R2', mentions_cleared: 1, mentions: null } }, 0)
    s = applyEvent(s, { t: 'MessageDeleted', d: { channel_id: 'Q1', message_id: 'R2', author_id: 'U2', mentions: ['U1'], thread_root_id: 'M1' } }, 0)
    expect(s.reads['Q1'].mentions).toBe(0)
  })

  it('an edited quote keeps my award flags and replies seen from the event derived awarders', () => {
    let s = withQuote({ lines: [line('L1', { awards: 1, awarders: ['U1'], awarded: true }), line('L2')], replies_seen: 'R1', last_reply_id: 'R1', reply_count: 1 })
    s = applyEvent(s, { t: 'MessageUpdated', d: { message: qmsg('M1', { lines: [line('L1', { awards: 1, awarders: ['U1'] }), line('L2', { text: 'edited' })], replies_seen: null, last_reply_id: 'R1', reply_count: 1 }) } }, 0)
    const q = quoteOf(s)
    expect(q.lines.map((l) => [l.awarded, l.text])).toEqual([[true, 'hi'], [false, 'edited']])
    expect(q.replies_seen).toBe('R1')
  })

  it('a link change republishes counts: awarded follows the awarders, not the old flag', () => {
    let s = withQuote({ lines: [line('L1', { awards: 1, awarders: ['U1'], awarded: true })] })
    s = applyEvent(s, { t: 'MessageUpdated', d: { message: qmsg('M1', { lines: [line('L1', { awards: 0, awarders: [] })] }) } }, 0)
    expect(quoteOf(s).lines[0]).toMatchObject({ awards: 0, awarded: false })
  })

  it('an edited quote keeps its reactions', () => {
    let s = withQuote()
    s = applyEvent(s, { t: 'ReactionAdded', d: { channel_id: 'Q1', message_id: 'M1', emoji: '👍', user_id: 'U1', count: 1 } }, 0)
    s = applyEvent(s, { t: 'MessageUpdated', d: { message: qmsg('M1') } }, 0)
    expect(s.messages['Q1'][0].reactions).toEqual([{ emoji: '👍', count: 1, me: true }])
  })

  it('awards update from events and mark mine', () => {
    let s = withQuote({ lines: [line('L1'), line('L2')] })
    const award = (user_id: string, given: boolean, awarders: string[]): Event => ({
      t: 'QuoteAwardChanged', d: { channel_id: 'Q1', message_id: 'M1', line_id: 'L1', user_id, given, count: awarders.length, awarders },
    })
    s = applyEvent(s, award('U2', true, ['U2']), 0)
    expect(quoteOf(s).lines[0]).toMatchObject({ awards: 1, awarders: ['U2'], awarded: false })
    s = applyEvent(s, award('U1', true, ['U2', 'U1']), 0)
    expect(quoteOf(s).lines[0]).toMatchObject({ awards: 2, awarders: ['U2', 'U1'], awarded: true })
    s = applyEvent(s, award('U2', false, ['U1']), 0) // the list is replaced, not appended to
    expect(quoteOf(s).lines[0]).toMatchObject({ awards: 1, awarders: ['U1'], awarded: true })
    expect(quoteOf(s).lines[1]).toMatchObject({ awards: 0, awarded: false })
  })

  it('an award on a quote that is not loaded changes nothing', () => {
    const s = qready()
    const e: Event = { t: 'QuoteAwardChanged', d: { channel_id: 'Q1', message_id: 'M9', line_id: 'L1', user_id: 'U2', given: true, count: 1, awarders: ['U2'] } }
    expect(applyEvent(s, e, 0)).toBe(s)
  })

  it('qoty events replace the server\'s event', () => {
    const ev = (over: Partial<QotyEvent> = {}): QotyEvent => ({
      id: 'E1', server_id: 'S1', year: 2026, started_by: 'U1', started_at: '2026-10-01T00:00:00.000Z', ends_at: null,
      ended_at: null, ended_by: null, end_reason: null, reopen_until: null, options: [], voters: 0, members: 2, ...over,
    })
    let s = applyReady(emptyState(), ready({ qoty: [ev({ year: 2025, id: 'E0' })] }))
    expect(s.qoty['S1'].id).toBe('E0')
    s = applyEvent(s, { t: 'QotyStarted', d: { event: ev() } }, 0)
    expect(s.qoty['S1'].id).toBe('E1')
    s = applyEvent(s, { t: 'QotyUpdated', d: { event: ev({ voters: 1 }) } }, 0)
    expect(s.qoty['S1'].voters).toBe(1)
    s = applyEvent(s, { t: 'QotyEnded', d: { event: ev({ ended_at: '2026-10-02T00:00:00.000Z', end_reason: 'early' }) } }, 0)
    expect(s.qoty['S1'].end_reason).toBe('early')
    expect(Object.keys(s.qoty)).toEqual(['S1'])
  })

  it('after a reconnect, a live vote wins over a newer ended one', () => {
    const ev = (over: Partial<QotyEvent> = {}): QotyEvent => ({
      id: 'E1', server_id: 'S1', year: 2026, started_by: 'U1', started_at: '2026-10-01T00:00:00.000Z', ends_at: null,
      ended_at: null, ended_by: null, end_reason: null, reopen_until: null, options: [], voters: 0, members: 2, ...over,
    })
    // 2026 ended early, 2025 started and ended, then 2026 reopened
    const live = ev()
    const older = ev({ id: 'E0', year: 2025, started_at: '2026-10-02T00:00:00.000Z', ended_at: '2026-10-02T01:00:00.000Z', end_reason: 'early' })
    expect(applyReady(emptyState(), ready({ qoty: [live, older] })).qoty['S1'].id).toBe('E1')
    expect(applyReady(emptyState(), ready({ qoty: [older, live] })).qoty['S1'].id).toBe('E1')
    // with nothing live, the newest still wins
    const ended = { ...live, ended_at: '2026-10-01T05:00:00.000Z' }
    expect(applyReady(emptyState(), ready({ qoty: [ended, older] })).qoty['S1'].id).toBe('E0')
  })

  describe('QotY options are live copies of their quotes', () => {
    const ev = (options: QotyEvent['options']): QotyEvent => ({
      id: 'E1', server_id: 'S1', year: 2026, started_by: 'U1', started_at: '2026-10-01T00:00:00.000Z', ends_at: null,
      ended_at: null, ended_by: null, end_reason: null, reopen_until: null, options, voters: 0, members: 2,
    })
    const line = { id: 'L1', speaker_id: 'U2', speaker_name: null, text: 'hi', media: null, caption: null, awards: 1, awarders: ['U1'], awarded: false }
    const q = (text = 'hi', reactions: Message['reactions'] = []): Message => ({
      ...msg('M1', 'Q1', `“${text}”`), reactions,
      quote: { lines: [{ ...line, text }], year: 2026, said_on: null, reply_count: 0, last_reply_id: null, replies_seen: null, wins: [] },
    })
    const optOf = (s: AppState) => s.qoty['S1'].options[0].message
    const base = () => applyReady(emptyState(), ready({ channels: [ch('Q1', 'quotes')], qoty: [ev([{ message: q(), added_by: null, votes: 0, voters: [] }])] }))

    it('my awards show as mine on Ready and every Qoty event', () => {
      expect(optOf(base()).quote!.lines[0].awarded).toBe(true)
      for (const t of ['QotyStarted', 'QotyUpdated', 'QotyEnded'] as const) {
        const s = applyEvent(applyReady(emptyState(), ready()), { t, d: { event: ev([{ message: q(), added_by: null, votes: 0, voters: [] }]) } } as Event, 0)
        expect(optOf(s).quote!.lines[0].awarded, t).toBe(true)
      }
    })

    it('reactions, edits and awards reach the option even when the channel is not loaded', () => {
      let s = base()
      expect(s.messages['Q1']).toBeUndefined()
      s = applyEvent(s, { t: 'ReactionAdded', d: { channel_id: 'Q1', message_id: 'M1', emoji: '😂', user_id: 'U2', count: 2 } }, 0)
      expect(optOf(s).reactions).toEqual([{ emoji: '😂', count: 2, me: false }])
      s = applyEvent(s, { t: 'MessageUpdated', d: { message: q('fixed') } }, 0)
      expect(optOf(s).quote!.lines[0].text).toBe('fixed')
      expect(optOf(s).quote!.lines[0].awarded).toBe(true)
      expect(optOf(s).reactions).toHaveLength(1) // an edit's reactions are not trusted
      s = applyEvent(s, { t: 'QuoteAwardChanged', d: { channel_id: 'Q1', message_id: 'M1', line_id: 'L1', user_id: 'U1', given: false, count: 0, awarders: [] } }, 0)
      expect(optOf(s).quote!.lines[0]).toMatchObject({ awards: 0, awarders: [], awarded: false })
      s = applyEvent(s, { t: 'ReactionRemoved', d: { channel_id: 'Q1', message_id: 'M1', emoji: '😂', user_id: 'U2', count: 0 } }, 0)
      expect(optOf(s).reactions).toEqual([])
    })

    it('a change to another message leaves the vote as it is', () => {
      const s = base()
      expect(applyEvent(s, { t: 'ReactionAdded', d: { channel_id: 'Q1', message_id: 'M9', emoji: '😂', user_id: 'U2', count: 1 } }, 0).qoty).toBe(s.qoty)
    })
  })

  it('applyMyVote moves only my vote, keeping newer votes from others', () => {
    const o = (id: string, voters: string[]) => ({ message: msg(id, 'Q1'), added_by: null, votes: voters.length, voters })
    const ev = (options: QotyEvent['options'], voters: number, id = 'E1'): QotyEvent => ({
      id, server_id: 'S1', year: 2026, started_by: 'U1', started_at: '2026-10-01T00:00:00.000Z', ends_at: null,
      ended_at: null, ended_by: null, end_reason: null, reopen_until: null, options, voters, members: 3,
    })
    // newer state already has sam's vote on M2; the answer to my vote predates it
    let s = applyEvent(applyReady(emptyState(), ready()), { t: 'QotyUpdated', d: { event: ev([o('M1', []), o('M2', ['U2'])], 1) } }, 0)
    s = applyMyVote(s, ev([o('M1', ['U1']), o('M2', [])], 1))
    expect(s.qoty['S1'].options.map((x) => [x.votes, x.voters])).toEqual([[1, ['U1']], [1, ['U2']]])
    expect(s.qoty['S1'].voters).toBe(2)
    // the same answer again changes nothing
    expect(applyMyVote(s, ev([o('M1', ['U1']), o('M2', [])], 1))).toBe(s)
    // moved, then taken back
    s = applyMyVote(s, ev([o('M1', []), o('M2', ['U2', 'U1'])], 2))
    expect(s.qoty['S1'].options.map((x) => x.voters)).toEqual([[], ['U2', 'U1']])
    s = applyMyVote(s, ev([o('M1', []), o('M2', ['U2'])], 1))
    expect(s.qoty['S1'].options.map((x) => x.voters)).toEqual([[], ['U2']])
    expect(s.qoty['S1'].voters).toBe(1)
    // an answer for another event (or none) is ignored
    expect(applyMyVote(s, ev([o('M1', ['U1'])], 1, 'E9'))).toBe(s)
    expect(applyMyVote(s, undefined)).toBe(s)
  })

  it('speaker links replace the server\'s list', () => {
    const link = { server_id: 'S1', name_key: 'sam', name: 'Sam', user_id: 'U2', linked_by: 'U1', linked_at: '2026-10-01T00:00:00.000Z' }
    let s = applyReady(emptyState(), ready({ speaker_links: [link, { ...link, server_id: 'S2', name_key: 'x' }] }))
    expect(s.speakerLinks['S1']).toEqual([link])
    expect(s.speakerLinks['S2']).toHaveLength(1)
    s = applyEvent(s, { t: 'SpeakerLinksChanged', d: { server_id: 'S1', links: [] } }, 0)
    expect(s.speakerLinks['S1']).toEqual([])
    expect(s.speakerLinks['S2']).toHaveLength(1)
  })

  it('history lists reach the latest by default', () => {
    expect(markHistory(emptyState(), 'C1', false).history['C1']).toEqual({ loaded: true, start: false, end: true })
  })

  it('removing a channel drops its threads even when their quotes are not loaded', () => {
    let s = created(qready(), reply('R1', 'M1', { mentions: ['U1'] })) // M1 itself is not loaded; a counted mention is kept
    // (seen up to before it: a seen point past it would drop it already, see "r1 m3")
    s = applyEvent(s, { t: 'ThreadSeen', d: { user_id: 'U1', channel_id: 'Q1', root_id: 'M1', last_seen_id: 'R0', mentions_cleared: 0, mentions: null } }, 0)
    s = applyEvent(s, { t: 'ChannelRemoved', d: { channel_id: 'Q1', user_id: 'U1' } }, 0)
    expect(s.threads['M1']).toBeUndefined()
    expect(s.threadSeen['M1']).toBeUndefined()
  })

  it('a new Ready clears the threads so a panel refetches', () => {
    let s = addReplies(created(withQuote(), reply('R1', 'M1')), 'M1', [reply('R1', 'M1')], true)
    expect(s.threadStart['M1']).toBe(true)
    s = applyReady(s, ready({ channels: [ch('Q1', 'quotes')] }))
    expect(s.threads).toEqual({})
    expect(s.threadStart).toEqual({})
  })

  it('addReplies merges a fetched page and remembers whether the start is reached', () => {
    let s = addReplies(withQuote(), 'M1', [reply('R2', 'M1'), reply('R3', 'M1')], false)
    expect(s.threadStart['M1']).toBe(false)
    s = created(s, reply('R4', 'M1'))
    s = addReplies(s, 'M1', [reply('R1', 'M1')], true)
    expect(s.threads['M1'].map((m) => m.id)).toEqual(['R1', 'R2', 'R3', 'R4'])
    expect(s.threadStart['M1']).toBe(true)
  })

  it('a live reply alone does not mark its thread as fetched', () => {
    expect(created(withQuote(), reply('R1', 'M1')).threadStart['M1']).toBeUndefined()
  })

  it('reactions on a reply update the reply in its thread', () => {
    let s = created(fetched(withQuote()), reply('R1', 'M1'))
    s = applyEvent(s, { t: 'ReactionAdded', d: { channel_id: 'Q1', message_id: 'R1', emoji: '👍', user_id: 'U1', count: 1 } }, 0)
    expect(s.threads['M1'][0].reactions).toEqual([{ emoji: '👍', count: 1, me: true }])
    s = applyEvent(s, { t: 'ReactionRemoved', d: { channel_id: 'Q1', message_id: 'R1', emoji: '👍', user_id: 'U1', count: 0 } }, 0)
    expect(s.threads['M1'][0].reactions).toEqual([])
    expect(s.messages['Q1'].map((m) => m.id)).toEqual(['M1'])
  })

  it('removing a channel drops the threads of its quotes', () => {
    let s = created(fetched(withQuote()), reply('R1', 'M1'))
    s = applyEvent(s, { t: 'ChannelRemoved', d: { channel_id: 'Q1', user_id: 'U1' } }, 0)
    expect(s.threads['M1']).toBeUndefined()
  })

  // --- fix wave U4 ---
  const unsentReply = (nonce: string, root: string | null = 'M1', status: Pending['status'] = 'failed'): Pending =>
    ({ nonce, content: `reply ${nonce}`, reply_to_id: null, thread_root_id: root, status, created_at: '2026-10-01T00:00:00.000Z' })
  const delRoot = (id = 'M1'): Event => ({ t: 'MessageDeleted', d: { channel_id: 'Q1', message_id: id, author_id: 'U2', mentions: [], thread_root_id: null } })

  it('I5: deleting a quote drops the unsent replies to it (they can never post), and only those', () => {
    let s = created(withQuote(), qmsg('M2'))
    s = addPending(s, 'Q1', unsentReply('a'))
    s = addPending(s, 'Q1', unsentReply('b', 'M2'))
    s = addPending(s, 'Q1', unsentReply('c', null))
    s = applyEvent(s, delRoot(), 0)
    expect(s.pending['Q1'].map((p) => p.nonce)).toEqual(['b', 'c'])
  })

  it('I5: restoring unsent replies drops those whose quote is known deleted', () => {
    const s = applyEvent(created(withQuote(), qmsg('M2')), delRoot(), 0)
    const r = restoreUnsent(s, { Q1: [unsentReply('a'), unsentReply('b', 'M2'), unsentReply('c', 'M9')] })
    expect(r.pending['Q1'].map((p) => p.nonce)).toEqual(['b', 'c']) // M9 isn't loaded: not known deleted
  })

  it('I5: a page that shows a quote deleted drops the unsent replies to it', async () => {
    const { addHistory, loadWindow } = await import('./state')
    const dead = { ...qmsg('M1'), deleted: true }
    const base = addPending(addPending(qready(), 'Q1', unsentReply('a')), 'Q1', unsentReply('b', 'M2'))
    expect(addHistory(base, 'Q1', [dead]).pending['Q1'].map((p) => p.nonce)).toEqual(['b'])
    expect(loadWindow(base, 'Q1', [dead, qmsg('M2')], 'M1').pending['Q1'].map((p) => p.nonce)).toEqual(['b'])
  })

  it('M13: a live reply to a thread that was never fetched is not kept, but still counts once', () => {
    let s = addPending(withQuote(), 'Q1', { nonce: 'n', content: 'ok', reply_to_id: null, thread_root_id: 'M1', status: 'pending', created_at: '2026-10-01T00:00:00.000Z' })
    s = created(s, reply('R1', 'M1', { author_id: 'U1' }), 'n') // the outbox
    s = created(s, reply('R1', 'M1', { author_id: 'U1' })) // and the gateway
    expect(s.threads['M1']).toBeUndefined()
    expect(quoteOf(s)).toMatchObject({ reply_count: 1, last_reply_id: 'R1' })
    expect(s.pending['Q1']).toEqual([])
  })

  it('M13: your reply confirmed after a newer one from someone else still counts', () => {
    let s = addPending(withQuote(), 'Q1', { nonce: 'n', content: 'ok', reply_to_id: null, thread_root_id: 'M1', status: 'pending', created_at: '2026-10-01T00:00:00.000Z' })
    s = created(s, reply('R2', 'M1')) // sam's, over the gateway
    s = created(s, reply('R1', 'M1', { author_id: 'U1' }), 'n') // yours, confirmed late over REST
    expect(quoteOf(s)).toMatchObject({ reply_count: 2, last_reply_id: 'R2' })
    s = created(s, reply('R1', 'M1', { author_id: 'U1' }), 'n') // and its gateway copy
    expect(quoteOf(s).reply_count).toBe(2)
  })

  it('M13: a counted mention in a thread never fetched is kept, so its delete still takes it back out', () => {
    let s = created(qready(), reply('R1', 'M1', { author_id: 'U2', mentions: ['U1'] })) // M1 isn't loaded
    s = created(s, reply('R2', 'M1', { author_id: 'U2' })) // no mention: not kept
    expect(s.threads['M1'].map((m) => m.id)).toEqual(['R1'])
    expect(s.reads['Q1'].mentions).toBe(1)
    s = applyEvent(s, { t: 'MessageDeleted', d: { channel_id: 'Q1', message_id: 'R1', author_id: 'U2', mentions: ['U1'], thread_root_id: 'M1' } }, 0)
    expect(s.reads['Q1'].mentions).toBe(0)
  })

  it('M13: closing a thread drops what was fetched for it (opening it again refetches)', async () => {
    const { closeThread } = await import('./state')
    let s = addReplies(withQuote(), 'M1', [reply('R1', 'M1'), reply('R2', 'M1', { author_id: 'U2', mentions: ['U1'] })], true)
    // the open panel marks what it shows seen; a later mention arrives unseen
    s = applyEvent(s, { t: 'ThreadSeen', d: { user_id: 'U1', channel_id: 'Q1', root_id: 'M1', last_seen_id: 'R2', mentions_cleared: 1, mentions: null } }, 0)
    s = created(s, reply('R3', 'M1', { author_id: 'U2', mentions: ['U1'] }))
    s = closeThread(s, 'M1', 'Q1')
    expect(s.threadStart['M1']).toBeUndefined()
    // only an unseen counted mention stays (its delete must still lower the count)
    expect(s.threads['M1'].map((m) => m.id)).toEqual(['R3'])
    s = applyEvent(s, { t: 'ThreadSeen', d: { user_id: 'U1', channel_id: 'Q1', root_id: 'M1', last_seen_id: 'R3', mentions_cleared: 1, mentions: null } }, 0)
    s = closeThread(s, 'M1', 'Q1')
    expect(s.threads['M1']).toBeUndefined()
  })

  it("M13: trimming a channel drops its gone replies whose quotes aren't loaded any more", async () => {
    const { addHistory, trimHistory } = await import('./state')
    const old = Array.from({ length: 150 }, (_, i) => qmsg(`M${String(i).padStart(4, '0')}`))
    let s = markHistory(addHistory(qready(), 'Q1', old), 'Q1', true)
    s = { ...s, reads: { ...s.reads, Q1: { ...s.reads['Q1'], last_read_message_id: 'M0149' } } }
    const del = (root: string, id: string): Event => ({ t: 'MessageDeleted', d: { channel_id: 'Q1', message_id: id, author_id: 'U2', mentions: [], thread_root_id: root } })
    s = applyEvent(applyEvent(s, del('M0000', 'R1'), 0), del('M0149', 'R2'), 0)
    expect(Object.keys(s.goneReplies).sort()).toEqual(['R1', 'R2'])
    s = trimHistory(s, 'Q1')
    expect(Object.keys(s.goneReplies)).toEqual(['R2']) // M0000 was trimmed off
  })

  it('M16: the open thread keeps its quote current after the quote leaves the list, and sees it deleted', async () => {
    const { holdThread, closeThread, loadLatest } = await import('./state')
    let s = holdThread(withQuote({ reply_count: 1, last_reply_id: 'R1' }), qmsg('M1', { reply_count: 1, last_reply_id: 'R1' }))
    s = loadLatest(s, 'Q1', [qmsg('M5')]) // "Jump to present": M1 is no longer loaded
    expect(s.messages['Q1'].some((m) => m.id === 'M1')).toBe(false)
    s = applyEvent(s, { t: 'MessageUpdated', d: { message: { ...qmsg('M1'), quote: quote({ lines: [line('L1', { text: 'edited' })], reply_count: 1, last_reply_id: 'R1' }) } } }, 0)
    expect(s.openRoot?.quote?.lines[0].text).toBe('edited')
    s = applyEvent(s, { t: 'QuoteAwardChanged', d: { channel_id: 'Q1', message_id: 'M1', line_id: 'L1', user_id: 'U1', given: true, count: 1, awarders: ['U1'] } }, 0)
    expect(s.openRoot?.quote?.lines[0]).toMatchObject({ awards: 1, awarded: true })
    s = applyEvent(s, { t: 'ReactionAdded', d: { channel_id: 'Q1', message_id: 'M1', emoji: '👍', user_id: 'U2', count: 1 } }, 0)
    expect(s.openRoot?.reactions).toEqual([{ emoji: '👍', count: 1, me: false }])
    s = created(fetched(s), reply('R2', 'M1'))
    expect(s.openRoot?.quote).toMatchObject({ reply_count: 2, last_reply_id: 'R2' })
    s = applyEvent(s, delRoot(), 0)
    expect(s.openRoot?.deleted).toBe(true)
    expect(closeThread(s, 'M1', 'Q1').openRoot).toBeNull()
  })

  // --- U4 round 1 ---
  it('r1 I1: a live reply to the open quote is kept while its first page is still loading', async () => {
    const { holdThread } = await import('./state')
    let s = holdThread(withQuote(), qmsg('M1'))
    s = created(s, reply('R2', 'M1')) // lands between the request and the response
    s = addReplies(s, 'M1', [reply('R1', 'M1')], true) // the page, read before R2 existed
    expect(s.threads['M1'].map((m) => m.id)).toEqual(['R1', 'R2'])
  })

  it("r1 m2: closing a thread another panel replaced keeps its gone replies while it's loaded", async () => {
    const { holdThread, closeThread } = await import('./state')
    let s = addReplies(created(withQuote({ reply_count: 1, last_reply_id: 'R1' }), qmsg('M2')), 'M1', [], true)
    s = applyEvent(s, { t: 'MessageDeleted', d: { channel_id: 'Q1', message_id: 'R1', author_id: 'U2', mentions: [], thread_root_id: 'M1' } }, 0)
    s = holdThread(s, qmsg('M2')) // the panel moved on to M2 before M1's close ran
    s = closeThread(s, 'M1', 'Q1')
    expect(s.goneReplies['R1']).toBeDefined()
    expect(s.openRoot?.id).toBe('M2')
  })

  it('r1 m3: a ThreadSeen past the kept mentions of a thread not open here drops them', () => {
    let s = created(qready(), reply('R1', 'M1', { author_id: 'U2', mentions: ['U1'] }))
    s = created(s, reply('R2', 'M1', { author_id: 'U2', mentions: ['U1'] }))
    s = applyEvent(s, { t: 'ThreadSeen', d: { user_id: 'U1', channel_id: 'Q1', root_id: 'M1', last_seen_id: 'R1', mentions_cleared: 1, mentions: null } }, 0)
    expect(s.threads['M1'].map((m) => m.id)).toEqual(['R2'])
    s = applyEvent(s, { t: 'ThreadSeen', d: { user_id: 'U1', channel_id: 'Q1', root_id: 'M1', last_seen_id: 'R2', mentions_cleared: 1, mentions: null } }, 0)
    expect(s.threads['M1']).toBeUndefined()
    // a fetched (open) thread keeps everything
    s = addReplies(withQuote(), 'M1', [reply('R1', 'M1')], true)
    s = applyEvent(s, { t: 'ThreadSeen', d: { user_id: 'U1', channel_id: 'Q1', root_id: 'M1', last_seen_id: 'R1', mentions_cleared: 0, mentions: null } }, 0)
    expect(s.threads['M1'].map((m) => m.id)).toEqual(['R1'])
  })
})

describe('detached windows (jump to a message)', () => {
  const ids = (from: number, n: number) => Array.from({ length: n }, (_, i) => `M${String(from + i).padStart(4, '0')}`)
  // The server's pages are newest first.
  const newestFirst = (list: string[]) => list.map((id) => msg(id)).reverse()
  const withLatest = async () => {
    const { addHistory } = await import('./state')
    return markHistory(addHistory(applyReady(emptyState(), ready()), 'C1', newestFirst(ids(900, 50))), 'C1', false)
  }

  it('a loaded window is detached from the present', async () => {
    const { loadWindow } = await import('./state')
    const s = await withLatest()
    // a full window: 25 older, the target, 24 newer
    const w = loadWindow(s, 'C1', newestFirst(ids(100, 50)), 'M0125')
    expect(w.messages['C1'].map((m) => m.id)).toEqual(ids(100, 50)) // replaced, oldest..newest
    expect(w.history['C1']).toEqual({ loaded: true, start: false, end: false })
    // near the channel start (only 3 older) and near the latest (only 2 newer)
    expect(loadWindow(s, 'C1', newestFirst(ids(0, 28)), 'M0003').history['C1']).toEqual({ loaded: true, start: true, end: false })
    expect(loadWindow(s, 'C1', newestFirst(ids(975, 28)), 'M1000').history['C1']).toEqual({ loaded: true, start: false, end: true })
  })

  it('loading the latest page attaches the list again', async () => {
    const { loadWindow, loadLatest } = await import('./state')
    const w = loadWindow(await withLatest(), 'C1', newestFirst(ids(100, 50)), 'M0125')
    const s = loadLatest(w, 'C1', newestFirst(ids(900, 50)))
    expect(s.history['C1']).toEqual({ loaded: true, start: false, end: true })
    expect(s.messages['C1'].map((m) => m.id)).toEqual(ids(900, 50))
    expect(loadLatest(w, 'C1', newestFirst(ids(0, 7))).history['C1']).toEqual({ loaded: true, start: true, end: true })
  })

  it('a live message missed while paging forward is fetched before the present is reached', async () => {
    const { loadWindow, appendNewer, loadLatest } = await import('./state')
    let s = loadWindow(await withLatest(), 'C1', newestFirst(ids(100, 50)), 'M0125')
    // M2000 arrives live after the server answered the last `after` request, before it's applied
    s = applyEvent(s, { t: 'MessageCreated', d: { message: msg('M2000'), nonce: null } }, 0)
    s = appendNewer(s, 'C1', 'M0149', newestFirst(ids(150, 10))) // short, but without M2000
    expect(s.history['C1'].end).toBe(false) // one more page to fetch
    s = appendNewer(s, 'C1', 'M0159', [msg('M2000')])
    expect(s.messages['C1'].at(-1)!.id).toBe('M2000')
    expect(s.history['C1']).toEqual({ loaded: true, start: false, end: true })
    // the same for a latest page fetched before the live message
    let p = loadWindow(await withLatest(), 'C1', newestFirst(ids(100, 50)), 'M0125')
    p = applyEvent(p, { t: 'MessageCreated', d: { message: msg('M2000'), nonce: null } }, 0)
    p = loadLatest(p, 'C1', newestFirst(ids(900, 50)))
    expect(p.history['C1'].end).toBe(false)
  })

  it('a missed live message that is deleted again no longer holds the list back', async () => {
    const { loadWindow, loadLatest, appendNewer } = await import('./state')
    let s = loadWindow(await withLatest(), 'C1', newestFirst(ids(100, 50)), 'M0125')
    // e.g. a winner card hard-deleted when the vote is reopened: no page will ever bring it
    s = applyEvent(s, { t: 'MessageCreated', d: { message: msg('M2000'), nonce: null } }, 0)
    s = applyEvent(s, { t: 'MessageDeleted', d: { channel_id: 'C1', message_id: 'M2000', author_id: 'U2', mentions: [], thread_root_id: null } }, 0)
    expect(s.history['C1'].missed).toBeUndefined()
    expect(appendNewer(s, 'C1', 'M0149', newestFirst(ids(150, 10))).history['C1'].end).toBe(true)
    expect(loadLatest(s, 'C1', newestFirst(ids(900, 50))).history['C1'].end).toBe(true)
  })

  it('with two missed messages, deleting the newer still waits for the older', async () => {
    const { loadWindow, appendNewer } = await import('./state')
    let s = loadWindow(await withLatest(), 'C1', newestFirst(ids(100, 50)), 'M0125')
    s = applyEvent(s, { t: 'MessageCreated', d: { message: msg('M2000'), nonce: null } }, 0)
    s = applyEvent(s, { t: 'MessageCreated', d: { message: msg('M2001'), nonce: null } }, 0)
    s = applyEvent(s, { t: 'MessageDeleted', d: { channel_id: 'C1', message_id: 'M2001', author_id: 'U2', mentions: [], thread_root_id: null } }, 0)
    s = appendNewer(s, 'C1', 'M0149', newestFirst(ids(150, 10))) // short, without M2000
    expect(s.history['C1'].end).toBe(false)
    s = appendNewer(s, 'C1', 'M0159', [msg('M2000')])
    expect(s.history['C1']).toEqual({ loaded: true, start: false, end: true })
  })

  it('more missed messages than are kept cost one more page', async () => {
    const { loadWindow, appendNewer, MISSED_CAP } = await import('./state')
    let s = loadWindow(await withLatest(), 'C1', newestFirst(ids(100, 50)), 'M0125')
    const live = ids(2000, MISSED_CAP + 1)
    for (const id of live) s = applyEvent(s, { t: 'MessageCreated', d: { message: msg(id), nonce: null } }, 0)
    expect(s.history['C1'].missed).toHaveLength(MISSED_CAP)
    // every kept one deleted: the dropped oldest may still be out there
    for (const id of live.slice(1)) s = applyEvent(s, { t: 'MessageDeleted', d: { channel_id: 'C1', message_id: id, author_id: 'U2', mentions: [], thread_root_id: null } }, 0)
    s = appendNewer(s, 'C1', 'M0149', [])
    expect(s.history['C1'].end).toBe(false) // one more page, to be sure
    s = appendNewer(s, 'C1', 'M0149', [msg('M2000')])
    expect(s.history['C1']).toEqual({ loaded: true, start: false, end: true })
  })

  it("a detached window takes the server's unread count", async () => {
    const { loadWindow } = await import('./state')
    const w = loadWindow(await withLatest(), 'C1', newestFirst(ids(100, 50)), 'M0125')
    const s = applyEvent(w, { t: 'ReadStateUpdated', d: { user_id: 'U1', channel_id: 'C1', last_read_message_id: 'M0950', mentions: 0, unread: 7 } }, 0)
    expect(s.reads['C1']).toMatchObject({ last_read_message_id: 'M0950', unread: 7 })
  })

  it('appending newer pages reaches the present without duplicates', async () => {
    const { loadWindow, appendNewer } = await import('./state')
    let s = loadWindow(await withLatest(), 'C1', newestFirst(ids(100, 50)), 'M0125')
    // a stale page (the list moved on) is dropped
    expect(appendNewer(s, 'C1', 'M0100', newestFirst(ids(101, 50)))).toBe(s)
    s = appendNewer(s, 'C1', 'M0149', newestFirst(ids(149, 50))) // overlaps by one
    expect(s.messages['C1'].map((m) => m.id)).toEqual(ids(100, 99))
    expect(s.history['C1'].end).toBe(false)
    s = appendNewer(s, 'C1', 'M0198', newestFirst(ids(199, 10))) // short: the latest is reached
    expect(s.messages['C1'].map((m) => m.id)).toEqual(ids(100, 109))
    expect(s.history['C1']).toEqual({ loaded: true, start: false, end: true })
  })

  it('a live message in a detached window waits for the present', async () => {
    const { loadWindow } = await import('./state')
    const w = loadWindow(await withLatest(), 'C1', newestFirst(ids(100, 50)), 'M0125')
    const live = { ...msg('M2000'), mentions: ['U1'] }
    const s = applyEvent(w, { t: 'MessageCreated', d: { message: live, nonce: null } }, 0)
    expect(s.messages['C1']).toBe(w.messages['C1']) // no gap in the list
    expect(s.reads['C1']).toMatchObject({ unread: 1, mentions: 1 })
    // an edit of a message outside the window doesn't splice it in either
    const edited = applyEvent(s, { t: 'MessageUpdated', d: { message: { ...live, content: 'edited' } } }, 0)
    expect(edited.messages['C1']).toBe(w.messages['C1'])
    // an edit inside the window still applies
    const inside = applyEvent(s, { t: 'MessageUpdated', d: { message: { ...msg('M0110'), content: 'edited' } } }, 0)
    expect(inside.messages['C1'].find((m) => m.id === 'M0110')?.content).toBe('edited')
  })

  it('a detached list is never trimmed', async () => {
    const { loadWindow, appendNewer, trimHistory } = await import('./state')
    let s = loadWindow(await withLatest(), 'C1', newestFirst(ids(100, 50)), 'M0125')
    for (let p = 0; p < 3; p++) {
      const last = s.messages['C1'].at(-1)!.id
      s = appendNewer(s, 'C1', last, newestFirst(ids(Number(last.slice(1)) + 1, 50)))
    }
    expect(s.messages['C1']).toHaveLength(200)
    expect(trimHistory(s, 'C1')).toBe(s)
  })

  it('a detached window survives a reconnect', async () => {
    const { loadWindow } = await import('./state')
    const w = loadWindow(await withLatest(), 'C1', newestFirst(ids(100, 50)), 'M0125')
    const s = applyReady(w, ready())
    expect(s.messages['C1']).toBe(w.messages['C1'])
    expect(s.history['C1']).toEqual({ loaded: true, start: false, end: false })
  })
})

describe('clips', () => {
  const sam = { id: 'U2', username: 'sam', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
  const cm = (id: string, game: string | null = null, over: Partial<Message> = {}): Message => ({
    ...msg(id, 'K1', ''),
    clip: { thumb_hash: '', duration_ms: 1000, width: 1, height: 1, size: 1, game_id: game, reply_count: 0, last_reply_id: null, replies_seen: null },
    ...over,
  })
  const base = () => applyReady(emptyState(), ready({
    channels: [ch('K1', 'clips')],
    members: [{ server_id: 'S1', members: [{ user: me, nickname: null }, { user: sam, nickname: null }] }],
    read_states: [{ channel_id: 'K1', last_read_message_id: null, unread: 0, mentions: 0 }],
  }))
  const open = (s: AppState, filter: ClipFilter, items: Message[] = []) => loadFeedPage(openFeed(s, 'K1', filter), feedKey('K1', filter), items)
  const created = (s: AppState, m: Message) => applyEvent(s, { t: 'MessageCreated', d: { message: m, nonce: null } }, 0)
  const ids = (s: AppState, filter: ClipFilter) => s.clipFeeds[feedKey('K1', filter)].items.map((m) => m.id)

  it('a new clip lands in matching feeds only', () => {
    let s = open(open(open(base(), {}), { game: 'G1' }), { game: 'G2' })
    s = created(s, cm('M1', 'G1'))
    s = created(s, cm('M2', 'G1'))
    expect(ids(s, {})).toEqual(['M2', 'M1'])
    expect(ids(s, { game: 'G1' })).toEqual(['M2', 'M1'])
    expect(ids(s, { game: 'G2' })).toEqual([])
    s = created(s, cm('M2', 'G1'))
    expect(ids(s, {})).toEqual(['M2', 'M1'])
  })

  it('a clip counts as unread and stays out of the text history', () => {
    const s = created(base(), cm('M1'))
    expect(s.reads['K1'].unread).toBe(1)
    expect(s.messages['K1']).toBeUndefined()
  })

  it("editing a clip's game moves it between filtered feeds", () => {
    let s = open(open(base(), { game: 'G1' }), { game: 'G2' })
    s = created(s, cm('M1', 'G1'))
    s = applyEvent(s, { t: 'MessageUpdated', d: { message: cm('M1', 'G2', { content: 'new' }) } }, 0)
    expect(ids(s, { game: 'G1' })).toEqual([])
    expect(ids(s, { game: 'G2' })).toEqual(['M1'])
    s = applyEvent(s, { t: 'MessageUpdated', d: { message: cm('M1', 'G2', { content: 'newer' }) } }, 0)
    expect(s.clipFeeds[feedKey('K1', { game: 'G2' })].items[0].content).toBe('newer')
  })

  it('a deleted clip leaves every feed', () => {
    let s = open(open(base(), {}), { game: 'G1' })
    s = created(s, cm('M1', 'G1'))
    s = applyEvent(s, { t: 'MessageDeleted', d: { channel_id: 'K1', message_id: 'M1', thread_root_id: null, author_id: 'U2', mentions: [] } }, 0)
    expect(ids(s, {})).toEqual([])
    expect(ids(s, { game: 'G1' })).toEqual([])
    expect(s.reads['K1'].unread).toBe(0)
  })

  it('a feed page appends older clips and notes the start', () => {
    let s = open(base(), {}, [cm('M9'), cm('M8')])
    expect(s.clipFeeds[feedKey('K1', {})].start).toBe(true)
    const page = Array.from({ length: 30 }, (_, i) => cm(`M${String(50 - i)}`))
    s = loadFeedPage(s, feedKey('K1', {}), page)
    expect(s.clipFeeds[feedKey('K1', {})].start).toBe(false)
    expect(ids(s, {}).slice(0, 3)).toEqual(['M9', 'M8', 'M50'])
    expect(ids(s, {})).toHaveLength(32)
  })

  it('clip views default to wall and follow events', () => {
    let s = applyReady(emptyState(), ready({ clip_views: [{ channel_id: 'K1', view: 'list' }] }))
    expect(clipView(s, 'K1')).toBe('list')
    expect(clipView(s, 'K2')).toBe('wall')
    s = applyEvent(s, { t: 'ClipViewChanged', d: { user_id: 'U1', channel_id: 'K1', view: 'wall' } }, 0)
    expect(clipView(s, 'K1')).toBe('wall')
  })

  it('games arrive by event', () => {
    let s = applyReady(emptyState(), ready({ games: [{ id: 'G2', server_id: 'S1', name: 'Zed', created_by: null, created_at: '' }] }))
    const g = { id: 'G1', server_id: 'S1', name: 'Apex', created_by: null, created_at: '' }
    s = applyEvent(s, { t: 'GameAdded', d: { game: g } }, 0)
    s = applyEvent(s, { t: 'GameAdded', d: { game: g } }, 0)
    expect(s.games['S1'].map((x) => x.name)).toEqual(['Apex', 'Zed'])
  })

  it('a reply to a clip counts on the clip', () => {
    let s = open(base(), {}, [cm('M1')])
    s = created(s, { ...msg('R1', 'K1', 'ok'), thread_root_id: 'M1' })
    expect(s.clipFeeds[feedKey('K1', {})].items[0].clip).toMatchObject({ reply_count: 1, last_reply_id: 'R1' })
    s = applyEvent(s, { t: 'MessageDeleted', d: { channel_id: 'K1', message_id: 'R1', thread_root_id: 'M1', author_id: 'U2', mentions: [] } }, 0)
    expect(s.clipFeeds[feedKey('K1', {})].items[0].clip!.reply_count).toBe(0)
  })

  it('a job is stored until done', () => {
    const job = { id: 'J1', channel_id: 'K1', stage: 'uploading' as const, progress: 0.5, error: null, caption: '', game_id: null, kept_ms: 1, thumb_path: null, message_id: null }
    let s = setClipJob(emptyState(), job)
    expect(s.clipJobs['J1'].progress).toBe(0.5)
    s = setClipJob(s, { ...job, stage: 'done' })
    expect(s.clipJobs['J1']).toBeUndefined() // cancelled: gone
  })

  it('a posted job is kept until its card is let go (the clip shows in its place)', () => {
    const job = { id: 'J1', channel_id: 'K1', stage: 'posting' as const, progress: 1, error: null, caption: '', game_id: null, kept_ms: 1, thumb_path: null, message_id: null }
    let s = setClipJob(emptyState(), job)
    s = setClipJob(s, { ...job, stage: 'done', message_id: 'M9' })
    expect(s.clipJobs['J1']).toMatchObject({ stage: 'done', message_id: 'M9' })
    s = dropClipJob(s, 'J1')
    expect(s.clipJobs['J1']).toBeUndefined()
    expect(dropClipJob(s, 'J1')).toBe(s)
  })

  it('a clip delete takes it out of unread', () => {
    let s = created(base(), cm('M1'))
    s = created(s, cm('M2'))
    s = applyEvent(s, { t: 'MessageDeleted', d: { channel_id: 'K1', message_id: 'M1', thread_root_id: null, author_id: 'U2', mentions: [] } }, 0)
    expect(s.reads['K1'].unread).toBe(1)
  })

  it('an edit adds a clip to a feed it newly fits, once the feed reaches that far', () => {
    let s = open(base(), {}, [cm('M1', 'G1')])
    s = open(s, { game: 'G2' }, [])
    s = open(s, { game: 'G3' }, Array.from({ length: 30 }, (_, i) => cm(`M${String(90 - i)}`, 'G3')))
    s = applyEvent(s, { t: 'MessageUpdated', d: { message: cm('M1', 'G2') } }, 0)
    expect(ids(s, { game: 'G2' })).toEqual(['M1'])
    s = applyEvent(s, { t: 'MessageUpdated', d: { message: cm('M1', 'G3') } }, 0)
    expect(ids(s, { game: 'G3' })).not.toContain('M1') // older than the page reached
  })

  it('reactions reach a clip in the feeds and the open clip', () => {
    let s = open(base(), {}, [cm('M1')])
    s = holdThread(s, s.clipFeeds[feedKey('K1', {})].items[0])
    s = applyEvent(s, { t: 'ReactionAdded', d: { channel_id: 'K1', message_id: 'M1', emoji: '👍', user_id: 'U1', count: 1 } }, 0)
    expect(s.clipFeeds[feedKey('K1', {})].items[0].reactions).toEqual([{ emoji: '👍', count: 1, me: true }])
    expect(s.openRoot!.reactions).toEqual([{ emoji: '👍', count: 1, me: true }])
    s = applyEvent(s, { t: 'ReactionRemoved', d: { channel_id: 'K1', message_id: 'M1', emoji: '👍', user_id: 'U1', count: 0 } }, 0)
    expect(s.clipFeeds[feedKey('K1', {})].items[0].reactions).toEqual([])
  })

  it('an edit of the open clip keeps its seen point', () => {
    let s = open(base(), {}, [])
    const withSeen = { ...cm('M1'), clip: { ...cm('M1').clip!, replies_seen: 'R5' } }
    s = holdThread(s, withSeen)
    s = applyEvent(s, { t: 'MessageUpdated', d: { message: cm('M1', null, { content: 'new' }) } }, 0)
    expect(s.openRoot!.content).toBe('new')
    expect(s.openRoot!.clip!.replies_seen).toBe('R5')
  })

  it('a clip deleted while its page was on the way does not come back', () => {
    let s = openFeed(base(), 'K1', {})
    s = applyEvent(s, { t: 'MessageDeleted', d: { channel_id: 'K1', message_id: 'M1', thread_root_id: null, author_id: 'U2', mentions: [] } }, 0)
    s = loadFeedPage(s, feedKey('K1', {}), [cm('M2'), cm('M1')])
    expect(ids(s, {})).toEqual(['M2'])
    expect(s.clipFeeds[feedKey('K1', {})].gone).toBeUndefined()
  })

  it('ThreadSeen on a clip moves its seen point', () => {
    let s = open(base(), {}, [cm('M1')])
    s = applyEvent(s, { t: 'ThreadSeen', d: { user_id: 'U1', channel_id: 'K1', root_id: 'M1', last_seen_id: 'R2', mentions_cleared: 0, mentions: null } }, 0)
    expect(s.clipFeeds[feedKey('K1', {})].items[0].clip!.replies_seen).toBe('R2')
  })
})

describe('profiles', () => {
  const card = { banner_hash: null, banner_still: null, theme_top: '#3a3d46', theme_bottom: '#16171b', pronouns: null, bio: null }
  const stats = { voice_seconds: 90, member_since: '2026-01-01T00:00:00Z', top_reaction: null, times_renamed: 2 }
  const profileOf = (user: typeof me) => ({ user, card, stats, badges: [], name_archive: [] })
  const sam = { ...me, id: 'U2', username: 'sam' }

  it('ProfileUpdated updates people, members, me and a cached profile (its stats unchanged)', () => {
    let s = applyReady(emptyState(), ready({
      people: [{ user: sam, online: true, last_seen_at: null }],
      members: [{ server_id: 'S1', members: [{ user: me, nickname: null }, { user: sam, nickname: 'S' }] }, { server_id: 'S2', members: [{ user: sam, nickname: null }] }],
    }))
    s = setProfile(s, profileOf(sam))
    s = setProfile(s, profileOf(me))
    const newSam = { ...sam, avatar_hash: 'h1' }
    const newCard = { ...card, pronouns: 'they/them' }
    s = applyEvent(s, { t: 'ProfileUpdated', d: { user: newSam, card: newCard } }, 0)
    expect(s.people.U2.user).toEqual(newSam)
    expect(s.people.U2.online).toBe(true)
    expect(s.members.S1[1]).toEqual({ user: newSam, nickname: 'S' })
    expect(s.members.S2[0].user).toEqual(newSam)
    expect(s.profiles.U2.user).toEqual(newSam)
    expect(s.profiles.U2.card).toEqual(newCard)
    expect(s.profiles.U2.stats).toEqual(stats)
    const newMe = { ...me, avatar_hash: 'h2' }
    s = applyEvent(s, { t: 'ProfileUpdated', d: { user: newMe, card } }, 0)
    expect(s.me).toEqual(newMe)
    expect(s.members.S1[0].user).toEqual(newMe)
    expect(s.profiles.U1.user).toEqual(newMe)
  })

  it('ProfileUpdated for an unknown person adds nobody', () => {
    const s0 = applyReady(emptyState(), ready())
    const s = applyEvent(s0, { t: 'ProfileUpdated', d: { user: { ...sam, id: 'U9' }, card } }, 0)
    expect(s.people).toEqual(s0.people)
    expect(s.members).toEqual(s0.members)
    expect(s.profiles).toEqual({})
  })

  it('setProfile caches', () => {
    const p = profileOf(sam)
    expect(setProfile(emptyState(), p).profiles.U2).toBe(p)
  })

  it('a (re)connect clears cached profiles', () => {
    let s = setProfile(applyReady(emptyState(), ready()), profileOf(sam))
    s = applyReady(s, ready())
    expect(s.profiles).toEqual({})
  })
})
