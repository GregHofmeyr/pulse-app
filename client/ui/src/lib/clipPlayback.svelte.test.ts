// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { claim, elementVolume, loadVolume, playback, release, setMuted, setVolume } from './clipPlayback.svelte'
import { mixer, setNotch } from './mixer.svelte'

beforeEach(() => {
  localStorage.clear()
  release(claim('reset', () => {}))
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
  it('claiming again with the same token pauses nothing', () => {
    const a = vi.fn()
    const t = claim('A', a)
    claim('A', a, t)
    expect(a).not.toHaveBeenCalled()
  })
  it('two players of one message displace each other', () => {
    const a = vi.fn()
    claim('A', a)
    claim('A', vi.fn())
    expect(a).toHaveBeenCalledTimes(1)
  })
  it('release only clears your own claim', () => {
    const t = claim('A', vi.fn())
    release(Symbol('other'))
    expect(playback.nowPlaying).toBe('A')
    release(t)
    expect(playback.nowPlaying).toBeNull()
  })
  it('a displaced player releasing does not clear the new holder', () => {
    const old = claim('A', vi.fn())
    claim('A', vi.fn())
    release(old)
    expect(playback.nowPlaying).toBe('A')
  })
})

describe('volume', () => {
  it('is the mixer’s Clips notch (percent), saved in pulse.mixer', () => {
    setVolume(40)
    setMuted(true)
    expect(JSON.parse(localStorage.getItem('pulse.mixer')!).media).toEqual({ clips: 40, clipsMuted: true })
    expect(mixer.media.clips).toBe(40)
    playback.volume = 100
    playback.muted = false
    expect(mixer.media).toEqual({ clips: 100, clipsMuted: false })
    setVolume(40)
    setMuted(true)
    loadVolume()
    expect(playback.volume).toBe(40)
    expect(playback.muted).toBe(true)
  })
  it('clamps to 0..100 and follows the mixer', () => {
    setVolume(300)
    expect(playback.volume).toBe(100)
    setVolume(-1)
    expect(playback.volume).toBe(0)
    setNotch('media', 'clips', 65)
    expect(playback.volume).toBe(65)
  })
  it('migrates the old linear clip volume by loudness', () => {
    localStorage.setItem('pulse.clipVolume', '{"volume":0.5,"muted":true}')
    loadVolume()
    expect(playback.volume).toBe(85)
    expect(playback.muted).toBe(true)
  })
  it('elementVolume maps the percent onto <video>.volume on the mixer curve, never above 1', () => {
    expect(elementVolume(100)).toBe(1)
    expect(elementVolume(0)).toBe(0)
    expect(elementVolume(50)).toBeCloseTo(0.1, 4)
    expect(elementVolume(85)).toBeCloseTo(0.5, 2)
    expect(elementVolume(200)).toBe(1)
  })
})
