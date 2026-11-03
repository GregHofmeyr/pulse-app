// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { Event } from './protocol/Event'

const hooks: Array<(e: Event) => void> = []
vi.mock('./store.svelte', () => ({ onAppEvent: (h: (e: Event) => void) => (hooks.push(h), () => {}) }))
vi.mock('./voice.svelte', () => ({ voice: { channelId: 'V1' } }))
const play = vi.fn(async () => {})
vi.mock('./tauri', () => ({ soundboardCmd: { play: (...a: unknown[]) => (play as (...x: unknown[]) => Promise<void>)(...a) } }))

import { setSoundMute, setSoundboardVolume, soundboard, startSoundboardListening } from './soundboard.svelte'

const played = (user_id: string, channel_id = 'V1'): Event => ({ t: 'SoundPlayed', d: { channel_id, user_id, sound_id: 'A', media_hash: 'hA', volume: 80 } })

describe('soundboard listener', () => {
  beforeEach(() => {
    play.mockClear()
    hooks.length = 0
    localStorage.clear()
    setSoundboardVolume(100)
    soundboard.mutes = {}
    startSoundboardListening()
  })
  it('plays locally for my call with the sound and soundboard volumes', () => {
    hooks[0](played('U2'))
    expect(play).toHaveBeenCalledWith('hA', 100, 80, null)
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
    expect(play).toHaveBeenCalledWith('hA', 50, 80, null)
  })
})
