import { describe, expect, it } from 'vitest'
import { applyEvent, applyReady, emptyState } from './state'
import { channelsFor, displayName, isMember, typingNames, voiceOccupants } from './selectors'
import type { Ready } from './protocol/Ready'

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
