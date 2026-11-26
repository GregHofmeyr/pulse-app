// @vitest-environment jsdom
// Speaking rings (#651 §5.3–5.4): levels go straight onto the ring elements as --lvl (never through $state), and the
// core is asked for them only while a visible reactive ring watches.
import { flushSync } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const h = vi.hoisted(() => ({ handlers: {} as Record<string, (e: { payload: unknown }) => void>, invokes: [] as [string, unknown][] }))
vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(async (cmd: string, args?: unknown) => {
    h.invokes.push([cmd, args])
    return undefined
  }),
}))
vi.mock('@tauri-apps/api/event', () => ({
  listen: async (name: string, cb: (e: { payload: unknown }) => void) => {
    h.handlers[name] = cb
    return () => {}
  },
}))

import { apply, nextBucket, register, resetSpeed, setWatching, type Bucket } from './ringLevels.svelte'
import { clearRings, lightRing } from './soundRings.svelte'
import { startVoiceListening, voice } from './voice.svelte'

const watchCalls = () => h.invokes.filter(([c]) => c === 'watch_voice_rings').map(([, a]) => a)
const el = (style = 'bars') => {
  const e = document.createElement('span')
  e.dataset.style = style
  document.body.append(e)
  return e
}
let cleanups: (() => void)[] = []
const reg = (userId: string, e: HTMLElement) => {
  const off = register(userId, e)
  cleanups.push(off)
  return off
}
const setVisibility = (v: 'visible' | 'hidden') => {
  Object.defineProperty(document, 'visibilityState', { configurable: true, get: () => v })
  document.dispatchEvent(new Event('visibilitychange'))
}

describe('watching (#651 §5.3)', () => {
  beforeEach(() => {
    h.invokes.length = 0
    setVisibility('visible')
  })
  afterEach(() => {
    cleanups.forEach((c) => c())
    cleanups = []
  })

  it('one_watch_invoke_per_transition', () => {
    const a = el()
    const b = el()
    reg('U1', a)
    reg('U2', b)
    setWatching(a, true)
    setWatching(b, true)
    setWatching(a, true) // counted per element: no double count
    expect(watchCalls()).toEqual([{ on: true }])
    setWatching(a, false)
    expect(watchCalls()).toEqual([{ on: true }])
    setWatching(b, false)
    expect(watchCalls()).toEqual([{ on: true }, { on: false }])
  })

  it('unmounting_the_last_watcher_unwatches_once', () => {
    const a = el()
    const off = reg('U1', a)
    setWatching(a, true)
    off()
    off() // a second cleanup is harmless
    expect(watchCalls()).toEqual([{ on: true }, { on: false }])
    apply({ U1: 0.5 })
    expect(a.style.getPropertyValue('--lvl')).toBe('')
  })

  it('a_hidden_document_unwatches_and_returns', () => {
    const a = el()
    reg('U1', a)
    setWatching(a, true)
    setVisibility('hidden')
    expect(watchCalls()).toEqual([{ on: true }, { on: false }])
    setVisibility('visible')
    expect(watchCalls()).toEqual([{ on: true }, { on: false }, { on: true }])
    setWatching(a, false)
  })

  it('nothing_is_asked_while_hidden_even_when_a_ring_starts_watching', () => {
    const a = el()
    reg('U1', a)
    setVisibility('hidden')
    setWatching(a, true)
    expect(watchCalls()).toEqual([])
    setVisibility('visible')
    expect(watchCalls()).toEqual([{ on: true }])
    setWatching(a, false)
  })
})

describe('applying levels (#651 §5.4)', () => {
  afterEach(() => {
    cleanups.forEach((c) => c())
    cleanups = []
  })

  it('apply_sets_lvl_on_the_right_elements_without_touching_state', () => {
    const a1 = el()
    const a2 = el()
    const b = el()
    reg('U1', a1)
    reg('U1', a2)
    reg('U2', b)
    let runs = 0
    const stop = $effect.root(() => {
      $effect(() => {
        void voice.speaking
        void voice.stage
        runs++
      })
    })
    flushSync()
    apply({ U1: 0.45 })
    flushSync()
    expect(a1.style.getPropertyValue('--lvl')).toBe('0.45')
    expect(a2.style.getPropertyValue('--lvl')).toBe('0.45')
    expect(b.style.getPropertyValue('--lvl')).toBe('')
    expect(runs).toBe(1)
    stop()
  })

  it('an_empty_frame_zeroes_every_element', () => {
    const a = el()
    const b = el()
    reg('U1', a)
    reg('U2', b)
    apply({ U1: 0.8, U2: 0.3 })
    apply({})
    expect(a.style.getPropertyValue('--lvl')).toBe('0')
    expect(b.style.getPropertyValue('--lvl')).toBe('0')
  })

  it('an_empty_frame_leaves_rings_that_never_had_a_level_at_the_fallback', () => {
    // a soundboard-lit ring (no voice level) keeps its calm .5 loop when the last voice speaker stops
    const voiced = el()
    const sound = el()
    reg('U1', voiced)
    reg('U2', sound)
    apply({ U1: 0.8 })
    apply({})
    expect(voiced.style.getPropertyValue('--lvl')).toBe('0')
    expect(sound.style.getPropertyValue('--lvl')).toBe('')
  })

  it('a_soundboard_lit_ring_eases_back_to_the_fallback_not_to_zero', () => {
    vi.useFakeTimers()
    try {
      const a = el()
      const b = el()
      reg('U1', a)
      reg('U2', b)
      lightRing('U2', 5000) // U2's sound keeps playing after they stop talking
      apply({ U1: 0.5, U2: 0.6 })
      apply({ U1: 0.5 })
      expect(b.style.getPropertyValue('--lvl')).toBe('')
      apply({ U2: 0.7 })
      apply({})
      expect(b.style.getPropertyValue('--lvl')).toBe('')
      expect(a.style.getPropertyValue('--lvl')).toBe('0')
    } finally {
      clearRings()
      vi.useRealTimers()
    }
  })

  // Phase 2 review: someone who drops out of a frame eases down (the scale transition does it), never snaps
  it('a_speaker_missing_from_the_next_frame_eases_to_zero', () => {
    const a = el()
    const b = el()
    reg('U1', a)
    reg('U2', b)
    apply({ U1: 0.2, U2: 0.6 })
    apply({ U2: 0.65 })
    expect(a.style.getPropertyValue('--lvl')).toBe('0')
    expect(b.style.getPropertyValue('--lvl')).toBe('0.65')
  })

  it('speed_buckets_hold_250ms_before_switching', () => {
    let b: Bucket = { rate: 1, pending: null, since: 0 }
    b = nextBucket(b, 0.2, 0)
    expect(b.rate).toBe(1) // pending, not switched yet
    b = nextBucket(b, 0.2, 260)
    expect(b.rate).toBe(0.75)
    b = nextBucket(b, 0.9, 300)
    expect(b).toMatchObject({ rate: 0.75, pending: 1.5 })
    b = nextBucket(b, 0.9, 500)
    expect(b.rate).toBe(0.75)
    b = nextBucket(b, 0.9, 560)
    expect(b.rate).toBe(1.5)
    // a flicker back under 250 ms resets the pending switch
    b = nextBucket(b, 0.5, 600)
    expect(b.pending).toBe(1)
    b = nextBucket(b, 0.9, 700)
    expect(b.pending).toBe(null)
    b = nextBucket(b, 0.5, 900)
    b = nextBucket(b, 0.5, 1000)
    expect(b.rate).toBe(1.5)
    b = nextBucket(b, 0.5, 1150)
    expect(b.rate).toBe(1)
  })

  it('a_restarted_ring_gets_its_speed_set_again', () => {
    vi.useFakeTimers()
    try {
      vi.setSystemTime(0)
      const d = el('dots')
      d.dataset.ring = 'on'
      const rate = vi.fn()
      d.getAnimations = () => [{ updatePlaybackRate: rate } as unknown as Animation]
      reg('U1', d)
      apply({ U1: 0.9 })
      vi.setSystemTime(300)
      apply({ U1: 0.9 })
      expect(rate).toHaveBeenCalledTimes(1)
      // the ring went off: its CSS animations restart at 1x next time, so the bucket must too
      resetSpeed(d)
      vi.setSystemTime(400)
      apply({ U1: 0.9 })
      vi.setSystemTime(700)
      apply({ U1: 0.9 })
      expect(rate).toHaveBeenCalledTimes(2)
      expect(rate).toHaveBeenLastCalledWith(1.5)
    } finally {
      vi.useRealTimers()
    }
  })

  it('a_still_ring_keeps_no_speed_so_it_starts_right_when_it_moves', () => {
    vi.useFakeTimers()
    try {
      vi.setSystemTime(0)
      const d = el('dots')
      d.dataset.ring = 'still'
      const rate = vi.fn()
      d.getAnimations = () => [{ updatePlaybackRate: rate } as unknown as Animation]
      reg('U1', d)
      apply({ U1: 0.9 })
      vi.setSystemTime(300)
      apply({ U1: 0.9 })
      expect(rate).not.toHaveBeenCalled()
      d.dataset.ring = 'on'
      vi.setSystemTime(400)
      apply({ U1: 0.9 })
      vi.setSystemTime(700)
      apply({ U1: 0.9 })
      expect(rate).toHaveBeenCalledWith(1.5)
    } finally {
      vi.useRealTimers()
    }
  })

  it('a_dots_ring_changes_speed_without_a_jump', () => {
    vi.useFakeTimers()
    try {
      vi.setSystemTime(0)
      const d = el('dots')
      d.dataset.ring = 'on'
      const bars = el('bars')
      bars.dataset.ring = 'on'
      const rate = vi.fn()
      const barRate = vi.fn()
      d.getAnimations = () => [{ updatePlaybackRate: rate } as unknown as Animation]
      bars.getAnimations = () => [{ updatePlaybackRate: barRate } as unknown as Animation]
      reg('U1', d)
      reg('U1', bars)
      apply({ U1: 0.9 })
      vi.setSystemTime(100)
      apply({ U1: 0.9 })
      expect(rate).not.toHaveBeenCalled()
      vi.setSystemTime(300)
      apply({ U1: 0.9 })
      expect(rate).toHaveBeenCalledWith(1.5)
      expect(barRate).not.toHaveBeenCalled()
    } finally {
      vi.useRealTimers()
    }
  })
})

describe('the voice listener (#651 §5.4)', () => {
  it('speaking_levels_never_reaches_reduceStage_or_state', async () => {
    const off = await startVoiceListening()
    const a = el()
    const stop = register('U1', a)
    const stage = voice.stage
    const speaking = voice.speaking
    h.handlers['voice://event']({ payload: { kind: 'speaking_levels', levels: { U1: 0.35 } } })
    expect(voice.stage).toBe(stage)
    expect(voice.speaking).toBe(speaking)
    expect(a.style.getPropertyValue('--lvl')).toBe('0.35')
    stop()
    off()
  })
})
