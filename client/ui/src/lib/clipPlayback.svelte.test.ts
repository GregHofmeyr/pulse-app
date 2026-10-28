// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { claim, loadVolume, playback, release, setMuted, setVolume } from './clipPlayback.svelte'

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
