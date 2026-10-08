import { describe, expect, it } from 'vitest'
import { applyEvent, applyReady, emptyState, addPending } from './state'
import type { Ready } from './protocol/Ready'
import type { Message } from './protocol/Message'
import type { Channel } from './protocol/Channel'

const ch = (id: string, kind: Channel['kind'] = 'text', server_id: string | null = 'S1', position = 0): Channel => ({
  id, server_id, kind, name: id, position,
})
const msg = (id: string, channel_id = 'C1', content = 'hi', author_id: string | null = 'U2'): Message => ({
  id, channel_id, author_id, kind: 'normal', content, reply_to_id: null,
  created_at: '2026-10-01T00:00:00.000Z', edited_at: null, deleted: false, mentions: [],
})
const me = { id: 'U1', username: 'alex', avatar_hash: null }
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
    let s = addPending(base(), 'C1', { nonce: 'n1', content: 'hi', reply_to_id: null, status: 'pending' })
    expect(s.pending['C1']).toHaveLength(1)
    s = applyEvent(s, { t: 'MessageCreated', d: { message: msg('M1', 'C1', 'hi', 'U1'), nonce: 'n1' } }, 0)
    expect(s.pending['C1']).toHaveLength(0)
    expect(s.messages['C1']).toHaveLength(1)
  })

  it('updates and deletes in place', () => {
    let s = applyEvent(base(), { t: 'MessageCreated', d: { message: msg('M1'), nonce: null } }, 0)
    s = applyEvent(s, { t: 'MessageUpdated', d: { message: { ...msg('M1'), content: 'edited', edited_at: 'x' } } }, 0)
    expect(s.messages['C1'][0].content).toBe('edited')
    s = applyEvent(s, { t: 'MessageDeleted', d: { channel_id: 'C1', message_id: 'M1', author_id: 'U2', mentions: [] } }, 0)
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
    const joined = { t: 'MemberJoined', d: { server_id: 'S1', member: { user: { id: 'U2', username: 'sam', avatar_hash: null }, nickname: null } } } as const
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
    expect(s.history['C1']).toEqual({ loaded: true, start: true })
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
    ({ t: 'MessageDeleted', d: { channel_id: 'D1', message_id: id, author_id: author, mentions } }) as const

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
