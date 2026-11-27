// @vitest-environment jsdom
// Speaking rings (#651): the 8-ring cap (first eight to start animate, a newcomer never bumps one) and the viewer's
// "Show everyone's rings as Classic" setting.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => undefined) }))
vi.mock('@tauri-apps/api/event', () => ({ listen: async () => () => {} }))

import { clearRings, lightRing } from './soundRings.svelte'
import { isAnimating, nextOrder, RING_CAP } from './speakingRing.svelte'
import { voice } from './voice.svelte'

const ids = (n: number, from = 1) => Array.from({ length: n }, (_, i) => String(from + i))
const speak = (list: string[]) => (voice.speaking = new Set(list))
const animating = (list: string[]) => list.filter((id) => isAnimating(id))

describe('the ring cap (#651 §8.3)', () => {
  beforeEach(() => {
    speak([])
    clearRings()
  })
  afterEach(() => {
    speak([])
    clearRings()
  })

  it('nextOrder_keeps_the_start_order_and_appends_newcomers_sorted', () => {
    expect(nextOrder([], new Set(['b', 'a']))).toEqual(['a', 'b'])
    expect(nextOrder(['b', 'a'], new Set(['a', 'b', 'c']))).toEqual(['b', 'a', 'c'])
    expect(nextOrder(['b', 'a', 'c'], new Set(['c', 'a']))).toEqual(['a', 'c'])
  })

  it('the_first_eight_to_start_animate_and_a_newcomer_never_bumps_one', () => {
    expect(RING_CAP).toBe(8)
    const lit: string[] = []
    for (const id of ids(9)) {
      lit.push(id)
      speak([...lit])
      isAnimating(id) // a tile reads it as each person starts
    }
    expect(animating(ids(9))).toEqual(ids(8))
    // 3 stops: 9 takes the free place
    speak(lit.filter((id) => id !== '3'))
    expect(isAnimating('9')).toBe(true)
    // 10 starts: nobody animating drops to still, and 10 waits
    speak([...lit.filter((id) => id !== '3'), '10'])
    expect(animating(['1', '2', '4', '5', '6', '7', '8', '9'])).toHaveLength(8)
    expect(isAnimating('10')).toBe(false)
  })

  it('a_tile_mounted_mid_speech_ranks_by_start_order', () => {
    const lit: string[] = []
    for (const id of ids(9)) {
      lit.push(id)
      speak([...lit])
      isAnimating('1') // only tile 1 is on screen while they start
    }
    // tile 9 mounts now: it ranks ninth, not first
    expect(isAnimating('9')).toBe(false)
    expect(isAnimating('8')).toBe(true)
  })

  it('soundboard_lit_people_count_toward_the_cap', () => {
    vi.useFakeTimers()
    try {
      speak(ids(7))
      isAnimating('1')
      lightRing('S', 5000)
      isAnimating('S')
      speak([...ids(7), '8'])
      expect(isAnimating('S')).toBe(true)
      expect(isAnimating('8')).toBe(false)
    } finally {
      clearRings()
      vi.useRealTimers()
    }
  })
})

describe('Show everyone\'s rings as Classic (#651 §7.2)', () => {
  beforeEach(() => {
    vi.resetModules()
    localStorage.clear()
  })

  it('the_classic_setting_persists', async () => {
    const m = await import('./speakingRing.svelte')
    expect(m.ringPrefs.classic).toBe(false)
    m.setClassicRings(true)
    expect(m.ringPrefs.classic).toBe(true)
    expect(localStorage.getItem('pulse.classicRings')).toBe('true')
    vi.resetModules()
    const again = await import('./speakingRing.svelte')
    expect(again.ringPrefs.classic).toBe(true)
  })

  it('the_classic_setting_survives_blocked_storage', async () => {
    const get = vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => {
      throw new Error('blocked')
    })
    const set = vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
      throw new Error('blocked')
    })
    try {
      const m = await import('./speakingRing.svelte')
      expect(m.ringPrefs.classic).toBe(false)
      expect(() => m.setClassicRings(true)).not.toThrow()
      expect(m.ringPrefs.classic).toBe(true)
    } finally {
      get.mockRestore()
      set.mockRestore()
    }
  })
})
