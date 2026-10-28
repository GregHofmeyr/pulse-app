// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { claim, loadVolume, playback, release, setMuted, setVolume } from './clipPlayback.svelte'

beforeEach(() => {
  localStorage.clear()
  playback.nowPlaying = null
})

describe('claim', () => {
  it('pauses the previous clip through its callback', () => {
    const a = vi.fn()
    const b = vi.fn()
    claim('A', a)
    expect(playback.nowPlaying).toBe('A')
    claim('B', b)
    expect(a).toHaveBeenCalledTimes(1)
    expect(b).not.toHaveBeenCalled()
    expect(playback.nowPlaying).toBe('B')
  })
  it('claiming again as the same clip pauses nothing', () => {
    const a = vi.fn()
    claim('A', a)
    claim('A', a)
    expect(a).not.toHaveBeenCalled()
  })
  it('release only clears your own claim', () => {
    claim('A', vi.fn())
    release('B')
    expect(playback.nowPlaying).toBe('A')
    release('A')
    expect(playback.nowPlaying).toBeNull()
  })
})

describe('volume', () => {
  it('is saved and loaded under pulse.clipVolume', () => {
    setVolume(0.4)
    setMuted(true)
    expect(JSON.parse(localStorage.getItem('pulse.clipVolume')!)).toEqual({ volume: 0.4, muted: true })
    playback.volume = 1
    playback.muted = false
    loadVolume()
    expect(playback.volume).toBe(0.4)
    expect(playback.muted).toBe(true)
  })
  it('clamps to 0..1 and ignores malformed storage', () => {
    setVolume(3)
    expect(playback.volume).toBe(1)
    setVolume(-1)
    expect(playback.volume).toBe(0)
    localStorage.setItem('pulse.clipVolume', '{"volume":"loud","muted":5}')
    loadVolume()
    expect(playback.volume).toBe(1)
    expect(playback.muted).toBe(false)
    localStorage.setItem('pulse.clipVolume', 'not json')
    loadVolume()
    expect(playback.volume).toBe(1)
  })
})
