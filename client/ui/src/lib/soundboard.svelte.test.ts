// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { Event } from './protocol/Event'

const hooks: Array<(e: Event) => void> = []
const store = vi.hoisted(() => ({ app: { state: null as unknown } }))
vi.mock('./store.svelte', () => ({ onAppEvent: (h: (e: Event) => void) => (hooks.push(h), () => {}), app: store.app }))
const voiceState = vi.hoisted(() => ({ channelId: 'V1' as string | null, speaking: new Set<string>() }))
vi.mock('./voice.svelte', () => ({ voice: voiceState }))
const play = vi.fn(async () => {})
const warm = vi.fn(async () => {})
vi.mock('./tauri', () => ({
  soundboardCmd: {
    play: (...a: unknown[]) => (play as (...x: unknown[]) => Promise<void>)(...a),
    warm: (...a: unknown[]) => (warm as (...x: unknown[]) => Promise<void>)(...a),
  },
}))

import { callChanged, setSoundMute, setSoundboardVolume, soundboard, startSoundboardListening } from './soundboard.svelte'
import { emptyState } from './state'
import type { Sound } from './protocol/Sound'
import { mixer, reloadMixer, setNotch } from './mixer.svelte'

const played = (user_id: string, channel_id = 'V1', duration_ms = 1500): Event => ({ t: 'SoundPlayed', d: { channel_id, user_id, sound_id: 'A', media_hash: 'hA', volume: 80, duration_ms } })
const snd = (id: string, server_id = 'S1', duration_ms = 1000): Sound => ({
  id, server_id, name: id, emoji: null, emoji_id: null, media_hash: 'h' + id, volume: 100, duration_ms, added_by: null,
  created_at: '2026-10-01T00:00:00.000Z', plays: 0,
})
const flush = () => new Promise((r) => setTimeout(r, 0))

describe('soundboard listener', () => {
  beforeEach(() => {
    play.mockClear()
    warm.mockClear()
    store.app.state = emptyState()
    voiceState.channelId = 'V1'
    hooks.length = 0
    localStorage.clear()
    reloadMixer()
    soundboard.mutes = {}
    startSoundboardListening()
  })
  it('plays locally for my call with the Calls, soundboard and sound volumes', () => {
    hooks[0](played('U2'))
    expect(play).toHaveBeenCalledWith('hA', 100, 100, 80, null, 'A')
    setNotch('calls', 'master', 40)
    setSoundboardVolume(180)
    hooks[0](played('U2'))
    expect(play).toHaveBeenLastCalledWith('hA', 40, 180, 80, null, 'A')
  })
  it('the soundboard volume is the mixer’s Soundboard notch (0-200)', () => {
    setSoundboardVolume(250)
    expect(soundboard.volume_pct).toBe(200)
    expect(mixer.calls.soundboard).toBe(200)
    setNotch('calls', 'soundboard', 35)
    expect(soundboard.volume_pct).toBe(35)
    soundboard.volume_pct = 60
    expect(mixer.calls.soundboard).toBe(60)
  })
  it('a Calls master at 0 silences it', () => {
    setNotch('calls', 'master', 0)
    hooks[0](played('U2'))
    expect(play).not.toHaveBeenCalled()
  })
  it('stays silent for other calls, muted people and volume 0', () => {
    hooks[0](played('U2', 'V9'))
    setSoundMute('U3', true)
    hooks[0](played('U3'))
    setSoundMute('U3', false)
    setSoundboardVolume(0)
    hooks[0](played('U2'))
    expect(play).not.toHaveBeenCalled()
    setSoundboardVolume(50)
    hooks[0](played('U3'))
    expect(play).toHaveBeenCalledWith('hA', 100, 50, 80, null, 'A')
  })
  it('a DM or group call plays like a server voice channel, and Mute soundboard holds there too (#175)', () => {
    voiceState.channelId = 'DM1'
    hooks[0](played('U2', 'V1'))
    expect(play, 'not the call I am in').not.toHaveBeenCalled()
    hooks[0](played('U2', 'DM1'))
    expect(play).toHaveBeenCalledTimes(1)
    setSoundMute('U2', true)
    hooks[0](played('U2', 'DM1'))
    expect(play).toHaveBeenCalledTimes(1)
    setSoundMute('U2', false)
  })
})

describe('who played it', () => {
  beforeEach(() => {
    hooks.length = 0
    localStorage.clear()
    reloadMixer()
    soundboard.mutes = {}
    store.app.state = emptyState()
    voiceState.channelId = 'V1'
    voiceState.speaking = new Set()
    callChanged(null) // a fresh start: no rings
    startSoundboardListening()
  })

  it("lights the player's ring for the sound's length once it plays, without touching speaking", async () => {
    vi.useFakeTimers()
    try {
      hooks[0](played('U2', 'V1', 1500))
      expect(soundboard.playing.has('U2')).toBe(false) // not yet heard
      await vi.advanceTimersByTimeAsync(0)
      expect(soundboard.playing.has('U2')).toBe(true)
      expect(voiceState.speaking.size).toBe(0)
      await vi.advanceTimersByTimeAsync(1000)
      hooks[0](played('U2', 'V1', 1500)) // a second play keeps it lit to its own end
      await vi.advanceTimersByTimeAsync(1000)
      expect(soundboard.playing.has('U2')).toBe(true)
      await vi.advanceTimersByTimeAsync(600)
      expect(soundboard.playing.has('U2')).toBe(false)
    } finally {
      vi.useRealTimers()
    }
  })

  it('uses the stored length when an older server sends none, and skips muted people', async () => {
    vi.useFakeTimers()
    try {
      store.app.state = { ...emptyState(), sounds: { A: snd('A', 'S1', 300) } }
      hooks[0](played('U2', 'V1', 0))
      await vi.advanceTimersByTimeAsync(0)
      expect(soundboard.playing.has('U2')).toBe(true)
      await vi.advanceTimersByTimeAsync(350)
      expect(soundboard.playing.has('U2')).toBe(false)
      setSoundMute('U3', true)
      hooks[0](played('U3'))
      await vi.advanceTimersByTimeAsync(0)
      expect(soundboard.playing.has('U3')).toBe(false)
    } finally {
      vi.useRealTimers()
    }
  })

  it('a sound that fails to play lights nothing', async () => {
    play.mockImplementationOnce(async () => {
      throw 'That sound couldn’t be loaded.'
    })
    hooks[0](played('U2'))
    await flush()
    expect(soundboard.playing.has('U2')).toBe(false)
  })

  it('leaving the call clears the rings', async () => {
    hooks[0](played('U2', 'V1', 60_000))
    await flush()
    expect(soundboard.playing.has('U2')).toBe(true)
    callChanged(null)
    expect(soundboard.playing.size).toBe(0)
  })
})

describe('warm-up', () => {
  beforeEach(() => {
    warm.mockClear()
    hooks.length = 0
    const me = { id: 'U1' }
    store.app.state = {
      ...emptyState(),
      me,
      servers: [{ id: 'S1', name: 'Main', icon_hash: null }],
      members: { S1: [{ user: me }] },
      sounds: { A: snd('A'), B: snd('B'), X: snd('X', 'S9') },
      soundFavourites: ['B'],
    } as never
    callChanged(null)
  })

  it("joining a call warms your servers' sounds, favourites first", () => {
    callChanged('V1')
    expect(warm).toHaveBeenCalledWith(['hB', 'hA'])
    callChanged('V2') // switching calls: already warm
    expect(warm).toHaveBeenCalledTimes(1)
  })

  it('a sound added while you are in a call is warmed too', () => {
    voiceState.channelId = 'V1'
    startSoundboardListening()
    warm.mockClear()
    hooks[0]({ t: 'SoundCreated', d: { sound: snd('N') } } as Event)
    expect(warm).toHaveBeenCalledWith(['hN'])
    voiceState.channelId = null
    hooks[0]({ t: 'SoundCreated', d: { sound: snd('M') } } as Event)
    expect(warm).toHaveBeenCalledTimes(1)
  })
})
