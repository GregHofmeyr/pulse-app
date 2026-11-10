// @vitest-environment jsdom
// Join sounds (#121) through the voice listener: what a join sounds like for everyone in the call, you included.
import { beforeAll, beforeEach, describe, expect, it, vi } from 'vitest'

const h = vi.hoisted(() => ({
  handlers: {} as Record<string, (e: { payload: unknown }) => void>,
  calls: [] as Array<{ cmd: string; args: Record<string, unknown> }>,
  failPlay: false,
}))
vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(async (cmd: string, args?: Record<string, unknown>) => {
    h.calls.push({ cmd, args: args ?? {} })
    if (cmd === 'soundboard_play' && h.failPlay) throw 'could not load the sound'
    return undefined
  }),
}))
vi.mock('@tauri-apps/api/event', () => ({
  listen: async (name: string, cb: (e: { payload: unknown }) => void) => {
    h.handlers[name] = cb
    return () => {}
  },
}))

import { app, startListening } from './store.svelte'
import { emptyState, type AppState } from './state'
import { startVoiceListening, voice } from './voice.svelte'
import { reloadMixer, setNotch } from './mixer.svelte'
import { joinSounds, setJoinSoundMute } from './joinSounds.svelte'
import { soundboard } from './soundboard.svelte'
import type { Member } from './protocol/Member'

const user = (id: string) => ({ id, username: id, display_name: id, avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } })
const member = (id: string, join_sound?: string): Member => ({ user: user(id) as Member['user'], nickname: null, ...(join_sound ? { join_sound } : {}) })
const sound = { id: 'A', server_id: 'S1', name: 'airhorn', emoji: null, emoji_id: null, media_hash: 'hA', volume: 120, duration_ms: 6000, added_by: null, created_at: '2026-11-10T00:00:00.000Z', plays: 0 }
const play = { sound_id: 'A', media_hash: 'hA', volume: 120, duration_ms: 6000 }
const flags = { muted: false, deafened: false }

const event = (payload: unknown) => h.handlers['pulse://event']({ payload })
const joined = (user_id: string, channel_id = 'V1', join_sound?: typeof play) => event({ t: 'VoiceJoined', d: { channel_id, user_id, flags, ...(join_sound ? { join_sound } : {}) } })
const left = (user_id: string, channel_id = 'V1') => event({ t: 'VoiceLeft', d: { channel_id, user_id } })
const connection = (channel_id: string | null, conn: string) => h.handlers['voice://event']({ payload: { kind: 'state', channel_id, connection: conn, controls: flags } })
/** What was played, in order: cues by name, and soundboard plays (other commands, like the mixer's, left out). */
const plays = () => h.calls.filter((c) => c.cmd === 'play_sound' || c.cmd === 'soundboard_play')
const cmds = () => plays().map((c) => (c.cmd === 'play_sound' ? `cue:${c.args.name}` : c.cmd))
const flush = () => new Promise((r) => setTimeout(r, 0))

function state(): AppState {
  return {
    ...emptyState(),
    me: user('ME') as AppState['me'],
    sounds: { A: sound },
    members: { S1: [member('ME', 'A'), member('U2', 'A'), member('U3')] },
    channels: {
      V1: { id: 'V1', server_id: 'S1', kind: 'voice', name: 'Lounge', position: 0 } as AppState['channels'][string],
      DM1: { id: 'DM1', server_id: null, kind: 'dm', name: null, position: 0 } as AppState['channels'][string],
    },
  }
}

beforeAll(async () => {
  await startListening()
  await startVoiceListening()
})

beforeEach(() => {
  // out of any call (this also drops a held cue), then a clean slate
  connection(null, 'disconnected')
  localStorage.clear()
  reloadMixer()
  joinSounds.mutes = {}
  soundboard.mutes = {}
  h.failPlay = false
  app.state = state()
  h.calls.length = 0
})

describe('someone else joins your call', () => {
  beforeEach(() => {
    connection('V1', 'connected')
    h.calls.length = 0
  })

  it('with a join sound: everyone plays it through the soundboard path, at Calls × Join sounds × its volume, not the cue', async () => {
    setNotch('calls', 'master', 80)
    setNotch('calls', 'join_sounds', 150)
    joined('U2', 'V1', play)
    await flush()
    expect(cmds()).toEqual(['soundboard_play'])
    expect(plays()[0].args).toMatchObject({ hash: 'hA', callsPercent: 80, soundboardPercent: 150, soundPercent: 120, soundId: 'A', player: 'U2' })
    // the joiner's ring lights while it plays
    expect(soundboard.playing.has('U2')).toBe(true)
  })

  it('without one (or held back by the 30 s rule): the normal Join cue', () => {
    joined('U3')
    expect(cmds()).toEqual(['cue:join'])
  })

  it('Join sounds at 0 % plays the normal cue for everyone', () => {
    setNotch('calls', 'join_sounds', 0)
    joined('U2', 'V1', play)
    expect(cmds()).toEqual(['cue:join'])
  })

  it('a person whose join sound you muted joins with the normal cue; others keep theirs', async () => {
    setJoinSoundMute('U2', true)
    joined('U2', 'V1', play)
    joined('U4', 'V1', play)
    await flush()
    expect(cmds()).toEqual(['cue:join', 'soundboard_play'])
    expect(JSON.parse(localStorage.getItem('pulse.joinSounds.mutes')!)).toEqual({ U2: true })
  })

  it('a join sound that can’t be played becomes the cue', async () => {
    h.failPlay = true
    joined('U2', 'V1', play)
    await flush()
    expect(cmds()).toEqual(['soundboard_play', 'cue:join'])
  })

  it('leaving keeps the normal leave cue', () => {
    left('U2')
    expect(cmds()).toEqual(['cue:leave'])
  })

  it('a join in another call is silent', () => {
    joined('U2', 'V9', play)
    expect(cmds()).toEqual([])
  })
})

describe('you join', () => {
  it('with a join sound in that server: you hear it too, once, when the server announces your join', async () => {
    connection('V1', 'connecting')
    connection('V1', 'connected')
    expect(cmds(), 'the cue waits').toEqual([])
    joined('ME', 'V1', play)
    await flush()
    expect(cmds()).toEqual(['soundboard_play'])
    expect(plays()[0].args).toMatchObject({ hash: 'hA', player: 'ME' })
  })

  it('the announcement can come first', async () => {
    connection('V1', 'connecting')
    joined('ME', 'V1', play)
    connection('V1', 'connected')
    await flush()
    expect(cmds()).toEqual(['soundboard_play'])
  })

  it('back within 30 s (no sound on the announcement): the normal cue', () => {
    connection('V1', 'connected')
    joined('ME', 'V1')
    expect(cmds()).toEqual(['cue:join'])
  })

  it('with no announcement in time, the normal cue anyway', () => {
    vi.useFakeTimers()
    try {
      connection('V1', 'connected')
      vi.advanceTimersByTime(3000)
      expect(cmds()).toEqual(['cue:join'])
    } finally {
      vi.useRealTimers()
    }
  })

  it('a DM call, no join sound of yours, or Join sounds at 0 %: the cue at once', () => {
    connection('DM1', 'connected')
    expect(cmds()).toEqual(['cue:join'])
    connection(null, 'disconnected')
    app.state = { ...state(), members: { S1: [member('ME')] } }
    h.calls.length = 0
    connection('V1', 'connected')
    expect(cmds()).toEqual(['cue:join'])
    connection(null, 'disconnected')
    app.state = state()
    setNotch('calls', 'join_sounds', 0)
    h.calls.length = 0
    connection('V1', 'connected')
    expect(cmds()).toEqual(['cue:join'])
  })
})
