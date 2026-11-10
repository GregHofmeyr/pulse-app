// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { emptyState, applyEvent, type AppState } from './state'
import type { Sound } from './protocol/Sound'
import type { Member } from './protocol/Member'
import type { JoinSoundPlay } from './protocol/JoinSoundPlay'
import { hasJoinSound, holdOwnCue, joinSoundOf, loadJoinSoundMutes, ownJoin, peerJoin, saveJoinSoundMutes, type JoinAction } from './joinSounds'

const sound = (id: string, server_id = 'S1'): Sound => ({
  id, server_id, name: id, emoji: null, emoji_id: null, media_hash: 'h' + id, volume: 120, duration_ms: 6000, added_by: null,
  created_at: '2026-11-10T00:00:00.000Z', plays: 0,
})
const member = (id: string, join_sound?: string): Member => ({
  user: { id, username: id, display_name: id, avatar_hash: null, avatar_still: null, style: {} as Member['user']['style'] },
  nickname: null,
  ...(join_sound ? { join_sound } : {}),
})
function state(): AppState {
  const s = emptyState()
  return {
    ...s,
    sounds: { A: sound('A'), B: sound('B', 'S2') },
    members: { S1: [member('U1', 'A'), member('U2', 'A'), member('U3')], S2: [member('U1')] },
    channels: {
      ...s.channels,
      V1: { id: 'V1', server_id: 'S1', kind: 'voice' } as AppState['channels'][string],
      DM1: { id: 'DM1', server_id: null, kind: 'dm' } as AppState['channels'][string],
    },
  }
}
const play: JoinSoundPlay = { sound_id: 'A', media_hash: 'hA', volume: 120, duration_ms: 6000 }
const joined = (user: string, channel = 'V1', join_sound?: JoinSoundPlay) =>
  ({ t: 'VoiceJoined', d: { channel_id: channel, user_id: user, flags: { muted: false, deafened: false }, ...(join_sound ? { join_sound } : {}) } }) as never

describe('join sound of a member', () => {
  it('reads the member entry, and a sound no longer on the soundboard counts as none', () => {
    const s = state()
    expect(joinSoundOf(s, 'S1', 'U2')).toBe('A')
    expect(joinSoundOf(s, 'S1', 'U3')).toBeNull()
    expect(joinSoundOf(s, 'S2', 'U2')).toBeNull()
    const { A: _gone, ...sounds } = s.sounds
    expect(joinSoundOf({ ...s, sounds }, 'S1', 'U2')).toBeNull()
    expect(hasJoinSound(s, 'U2')).toBe(true)
    expect(hasJoinSound(s, 'U3')).toBe(false)
  })

  it('a deleted sound is cleared from everyone it was the join sound of', () => {
    const s = applyEvent(state(), { t: 'SoundDeleted', d: { server_id: 'S1', sound_id: 'A' } }, Date.now())
    expect(s.members.S1.map((m) => m.join_sound ?? null)).toEqual([null, null, null])
  })
})

describe('peerJoin', () => {
  const ctx = { myId: 'U1', channelId: 'V1', connection: 'connected' as const, level: 100, mutes: {} as Record<string, true> }
  it('someone joining your call with a join sound plays it', () => {
    expect(peerJoin(joined('U2', 'V1', play), ctx)).toEqual({ kind: 'sound', play })
  })
  it('falls back to the normal cue: no sound on the event (DM, 30 s rule, old server), Join sounds at 0 %, muted for you', () => {
    expect(peerJoin(joined('U2'), ctx)).toEqual({ kind: 'cue' })
    expect(peerJoin(joined('U2', 'V1', play), { ...ctx, level: 0 })).toEqual({ kind: 'cue' })
    expect(peerJoin(joined('U2', 'V1', play), { ...ctx, mutes: { U2: true } })).toEqual({ kind: 'cue' })
    expect(peerJoin(joined('U3', 'V1', play), { ...ctx, mutes: { U2: true } }), 'only that person').toEqual({ kind: 'sound', play })
  })
  it('not someone else joining your call: nothing (your own join, another channel, mid-join, leaves)', () => {
    expect(peerJoin(joined('U1', 'V1', play), ctx)).toBeNull()
    expect(peerJoin(joined('U2', 'V9', play), ctx)).toBeNull()
    expect(peerJoin(joined('U2', 'V1', play), { ...ctx, connection: 'connecting' })).toBeNull()
    expect(peerJoin(joined('U2', 'V1', play), { ...ctx, channelId: null })).toBeNull()
    expect(peerJoin({ t: 'VoiceLeft', d: { channel_id: 'V1', user_id: 'U2' } } as never, ctx)).toBeNull()
  })
})

describe('holdOwnCue', () => {
  it('only in a server voice channel where you have a join sound, with Join sounds above 0 %', () => {
    const s = state()
    expect(holdOwnCue(s, 'V1', 'U1', 100)).toBe(true)
    expect(holdOwnCue(s, 'V1', 'U3', 100), 'none set').toBe(false)
    expect(holdOwnCue(s, 'V1', 'U1', 0), 'Join sounds off').toBe(false)
    expect(holdOwnCue(s, 'DM1', 'U1', 100), 'DMs keep the cue').toBe(false)
    expect(holdOwnCue(s, 'V1', undefined, 100)).toBe(false)
  })
})

describe('ownJoin', () => {
  let acts: JoinAction[]
  let own: ReturnType<typeof ownJoin>
  beforeEach(() => {
    vi.useFakeTimers()
    acts = []
    own = ownJoin((a) => acts.push(a), 3000)
  })
  afterEach(() => vi.useRealTimers())

  it('not held: the caller plays the cue now, and a later announcement adds nothing', () => {
    expect(own.connected('V1', false)).toBe(false)
    own.announced('V1', play)
    vi.advanceTimersByTime(5000)
    expect(acts).toEqual([])
  })
  it('held, then announced with the sound: the sound, once', () => {
    expect(own.connected('V1', true)).toBe(true)
    own.announced('V1', play)
    vi.advanceTimersByTime(5000)
    expect(acts).toEqual([{ kind: 'sound', play }])
  })
  it('held, then announced without one (the 30 s rule): the normal cue, once', () => {
    own.connected('V1', true)
    own.announced('V1', null)
    vi.advanceTimersByTime(5000)
    expect(acts).toEqual([{ kind: 'cue' }])
  })
  it('the announcement first, then the connection: the sound at once', () => {
    own.announced('V1', play, Date.now())
    expect(own.connected('V1', true, Date.now() + 500)).toBe(true)
    vi.advanceTimersByTime(5000)
    expect(acts).toEqual([{ kind: 'sound', play }])
  })
  it('no announcement within the wait: the normal cue', () => {
    own.connected('V1', true)
    vi.advanceTimersByTime(2999)
    expect(acts).toEqual([])
    vi.advanceTimersByTime(1)
    expect(acts).toEqual([{ kind: 'cue' }])
  })
  it('an old or other-channel announcement is not this join', () => {
    own.announced('V9', play, Date.now())
    own.connected('V1', true)
    own.announced('V9', play)
    vi.advanceTimersByTime(3000)
    expect(acts).toEqual([{ kind: 'cue' }])
    acts = []
    own.announced('V1', play, Date.now() - 10_000)
    own.connected('V1', true, Date.now())
    vi.advanceTimersByTime(3000)
    expect(acts).toEqual([{ kind: 'cue' }])
  })
  it('moving to another call drops what waits for the one you left; what waits for the new one stays', () => {
    own.connected('V1', true)
    own.moved('V2')
    own.announced('V1', play)
    vi.advanceTimersByTime(5000)
    expect(acts, 'nothing for the call you left').toEqual([])
    own.announced('V2', play, Date.now())
    own.moved('V2')
    own.connected('V2', true)
    expect(acts).toEqual([{ kind: 'sound', play }])
  })

  it('says which call it acted for', () => {
    const seen: string[] = []
    const o = ownJoin((_a, channel) => seen.push(channel), 3000)
    o.connected('V1', true)
    o.announced('V1', null)
    o.announced('V2', play, Date.now())
    o.connected('V2', true)
    expect(seen).toEqual(['V1', 'V2'])
  })

  it('leaving drops whatever waits', () => {
    own.connected('V1', true)
    own.reset()
    vi.advanceTimersByTime(5000)
    expect(acts).toEqual([])
  })
})

describe('per-person mutes', () => {
  it('round-trip through storage and ignore junk', () => {
    localStorage.clear()
    expect(loadJoinSoundMutes()).toEqual({})
    saveJoinSoundMutes({ U2: true })
    expect(loadJoinSoundMutes()).toEqual({ U2: true })
    localStorage.setItem('pulse.joinSounds.mutes', '{"U3": 1, "U4": true}')
    expect(loadJoinSoundMutes()).toEqual({ U4: true })
    localStorage.setItem('pulse.joinSounds.mutes', 'nope')
    expect(loadJoinSoundMutes()).toEqual({})
  })
})
