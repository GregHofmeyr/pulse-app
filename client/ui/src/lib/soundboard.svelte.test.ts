// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { Event } from './protocol/Event'

const hooks: Array<(e: Event) => void> = []
vi.mock('./store.svelte', () => ({ onAppEvent: (h: (e: Event) => void) => (hooks.push(h), () => {}) }))
vi.mock('./voice.svelte', () => ({ voice: { channelId: 'V1' } }))
const play = vi.fn(async () => {})
vi.mock('./tauri', () => ({ soundboardCmd: { play: (...a: unknown[]) => (play as (...x: unknown[]) => Promise<void>)(...a) } }))

import { setSoundMute, setSoundboardVolume, soundboard, startSoundboardListening } from './soundboard.svelte'
import { mixer, reloadMixer, setNotch } from './mixer.svelte'

const played = (user_id: string, channel_id = 'V1'): Event => ({ t: 'SoundPlayed', d: { channel_id, user_id, sound_id: 'A', media_hash: 'hA', volume: 80 } })

describe('soundboard listener', () => {
  beforeEach(() => {
    play.mockClear()
    hooks.length = 0
    localStorage.clear()
    reloadMixer()
    soundboard.mutes = {}
    startSoundboardListening()
  })
  it('plays locally for my call with the Calls, soundboard and sound volumes', () => {
    hooks[0](played('U2'))
    expect(play).toHaveBeenCalledWith('hA', 100, 100, 80, null)
    setNotch('calls', 'master', 40)
    setSoundboardVolume(180)
    hooks[0](played('U2'))
    expect(play).toHaveBeenLastCalledWith('hA', 40, 180, 80, null)
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
    expect(play).toHaveBeenCalledWith('hA', 100, 50, 80, null)
  })
})
