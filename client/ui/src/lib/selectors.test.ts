import { describe, expect, it } from 'vitest'
import { applyEvent, applyReady, emptyState } from './state'
import { channelsFor, displayName, isLive, isMember, liveIn, typingNames, voiceOccupants } from './selectors'
import type { Ready } from './protocol/Ready'
import type { StreamInfo } from './protocol/StreamInfo'

const me = { id: 'U1', username: 'alex', avatar_hash: null }
const sam = { id: 'U2', username: 'sam', avatar_hash: null }
const r: Ready = {
  me,
  servers: [{ id: 'S1', name: 'Main', icon_hash: null }, { id: 'S2', name: 'Other', icon_hash: null }],
  channels: [
    { id: 'V1', server_id: 'S1', kind: 'voice', name: 'Lounge', position: 0 },
    { id: 'C2', server_id: 'S1', kind: 'text', name: 'memes', position: 1 },
    { id: 'C1', server_id: 'S1', kind: 'text', name: 'general', position: 0 },
    { id: 'X1', server_id: 'S2', kind: 'text', name: 'general', position: 0 },
    { id: 'D1', server_id: null, kind: 'dm', name: null, position: 0 },
  ],
  members: [{ server_id: 'S1', members: [{ user: me, nickname: null }, { user: sam, nickname: 'Big Dog' }] }, { server_id: 'S2', members: [{ user: sam, nickname: null }] }],
  dm_members: [],
  voice: [{ channel_id: 'V1', members: [{ user_id: 'U2', flags: { muted: true, deafened: false } }] }],
  people: [],
  read_states: [],
  mutes: [],
  hidden: [],
  latest: [],
  streams: [],
}
const s = applyReady(emptyState(), r)

describe('selectors', () => {
  it('channelsFor splits text/voice, sorted by position, only that server', () => {
    const c = channelsFor(s, 'S1')
    expect(c.text.map((x) => x.id)).toEqual(['C1', 'C2'])
    expect(c.voice.map((x) => x.id)).toEqual(['V1'])
  })

  it('isMember', () => {
    expect(isMember(s, 'S1')).toBe(true)
    expect(isMember(s, 'S2')).toBe(false)
  })

  it('displayName prefers the server nickname', () => {
    expect(displayName(s, 'S1', 'U2')).toBe('Big Dog')
    expect(displayName(s, 'S2', 'U2')).toBe('sam')
    expect(displayName(s, 'S1', 'U404')).toBe('unknown')
  })

  it('voiceOccupants resolves names and flags', () => {
    expect(voiceOccupants(s, 'S1', 'V1')).toEqual([{ userId: 'U2', name: 'Big Dog', muted: true, deafened: false }])
    expect(voiceOccupants(s, 'S1', 'nope')).toEqual([])
  })

  it('typingNames excludes me and expired entries', () => {
    let t = applyEvent(s, { t: 'Typing', d: { channel_id: 'C1', user_id: 'U2' } }, 1000)
    t = applyEvent(t, { t: 'Typing', d: { channel_id: 'C1', user_id: 'U1' } }, 1000)
    expect(typingNames(t, 'S1', 'C1', 2000)).toEqual(['Big Dog'])
    expect(typingNames(t, 'S1', 'C1', 8000)).toEqual([])
  })
})

describe('stableList', () => {
  it('keeps the old array when nothing changed', async () => {
    const { stableList } = await import('./selectors')
    const a = ['sam', 'alex']
    expect(stableList(a, ['sam', 'alex'])).toBe(a)
    const b = ['sam']
    expect(stableList(a, b)).toBe(b)
    expect(stableList(a, ['alex', 'sam'])).not.toBe(a)
  })
})

describe('nextTypingExpiry', () => {
  it('is the earliest live expiry, or null when nobody is typing', async () => {
    const { nextTypingExpiry } = await import('./selectors')
    let t = applyEvent(applyReady(emptyState(), r), { t: 'Typing', d: { channel_id: 'C1', user_id: 'U2' } }, 1000)
    t = applyEvent(t, { t: 'Typing', d: { channel_id: 'C1', user_id: 'U1' } }, 3000)
    expect(nextTypingExpiry(t, 'C1', 2000)).toBe(7000)
    expect(nextTypingExpiry(t, 'C1', 7000)).toBe(9000)
    expect(nextTypingExpiry(t, 'C1', 9000)).toBeNull()
    expect(nextTypingExpiry(t, 'C2', 0)).toBeNull()
  })
})

describe('stream selectors', () => {
  const stream: StreamInfo = { channel_id: 'V1', user_id: 'U2', settings: { height: 720, fps: 60, audio: true }, status: 'live', viewers: [], started_at: '2026-10-23T10:00:00.000Z' }
  it('liveIn lists a channel streams and isLive checks one user', () => {
    const s = { ...emptyState(), streams: { V1: { U2: stream } } }
    expect(liveIn(s, 'V1')).toEqual({ U2: stream })
    expect(liveIn(s, 'V9')).toEqual({})
    expect(isLive(s, 'V1', 'U2')).toBe(true)
    expect(isLive(s, 'V1', 'U3')).toBe(false)
    expect(isLive(s, 'V9', 'U2')).toBe(false)
  })
})
