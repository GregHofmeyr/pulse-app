// @vitest-environment jsdom
// Speaking rings (#651 §3.2, §3.5, §4): one component draws every ring. It goes off, on or still without creating DOM,
// renders each style's layers, and only a visible reactive ring asks the core for levels.
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const h = vi.hoisted(() => ({ watching: [] as [HTMLElement, boolean][] }))
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => undefined) }))
vi.mock('@tauri-apps/api/event', () => ({ listen: async () => () => {} }))
vi.mock('../lib/ringLevels.svelte', async (orig) => {
  const real = await orig<typeof import('../lib/ringLevels.svelte')>()
  return {
    ...real,
    setWatching: (el: HTMLElement, on: boolean) => {
      h.watching.push([el, on])
      real.setWatching(el, on)
    },
  }
})

import SpeakingRing from './SpeakingRing.svelte'
import { motion } from '../lib/animate.svelte'
import { ringPrefs } from '../lib/speakingRing.svelte'
import { app } from '../lib/store.svelte'
import { emptyState } from '../lib/state'
import type { User } from '../lib/protocol/User'
import { speakCss, type RingColor, type RingPlace, type RingStyle } from '../lib/speakingRing'
import { nameBg } from '../lib/theme.svelte'

const person = (ring_style?: string, ring_color?: string): User => ({
  id: 'U1',
  username: 'sam',
  display_name: 'sam',
  avatar_hash: null,
  avatar_still: null,
  style: { font: null, colors: [] },
  ...(ring_style ? { ring_style } : {}),
  ...(ring_color ? { ring_color } : {}),
})
const withUser = (u: User) => (app.state = { ...emptyState(), me: u })

let cmp: ReturnType<typeof mount> | undefined
let props: { userId: string; d: number; speaking: boolean; place: RingPlace; style?: RingStyle; color?: RingColor; user?: User }
function render(p: Partial<typeof props> = {}) {
  const fresh = $state({ userId: 'U1', d: 96, speaking: false, place: 'stage' as RingPlace, ...p })
  props = fresh
  cmp = mount(SpeakingRing, { target: document.body, props })
  flushSync()
  return document.querySelector<HTMLElement>('.ring-box')!
}

beforeEach(() => {
  withUser(person())
  Object.assign(motion, { reducedMotion: false, streaming: false })
  ringPrefs.classic = false
  h.watching.length = 0
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
  Object.assign(motion, { reducedMotion: false, streaming: false })
  ringPrefs.classic = false
})

describe('SpeakingRing (#651)', () => {
  it('off_on_still_follow_speaking_and_motion', () => {
    const box = render()
    expect(box.dataset.ring).toBe('off')
    props.speaking = true
    flushSync()
    expect(box.dataset.ring).toBe('on')
    motion.reducedMotion = true
    flushSync()
    expect(box.dataset.ring).toBe('still')
    motion.reducedMotion = false
    motion.streaming = true
    flushSync()
    expect(box.dataset.ring).toBe('still')
  })

  it('the_channel_list_is_always_still', () => {
    const box = render({ place: 'list', speaking: true, d: 22 })
    expect(box.dataset.ring).toBe('still')
  })

  it('sets_its_size_and_colour_on_the_root_without_forming_a_stacking_context', () => {
    withUser(person('pulse', 'gold'))
    const box = render({ d: 34, place: 'band' })
    expect(box.style.getPropertyValue('--d')).toBe('34px')
    expect(box.style.getPropertyValue('--speak')).toBe('var(--speak-gold)')
    expect(box.getAttribute('aria-hidden')).toBe('true')
    expect(box.dataset.style).toBe('pulse')
    expect(box.dataset.place).toBe('band')
  })

  it('starting_to_speak_creates_no_dom', () => {
    withUser(person('bars'))
    const box = render()
    const seen: MutationRecord[] = []
    const mo = new MutationObserver((r) => seen.push(...r))
    mo.observe(box, { childList: true, subtree: true })
    props.speaking = true
    flushSync()
    props.speaking = false
    flushSync()
    seen.push(...mo.takeRecords())
    mo.disconnect()
    expect(seen).toEqual([])
  })

  it('layer_counts_per_style', () => {
    const count = (style: string, d: number, sel: string) => {
      withUser(person(style))
      const box = render({ d })
      const n = box.querySelectorAll(sel).length
      unmount(cmp!)
      cmp = undefined
      return n
    }
    expect(count('dots', 96, '.dot')).toBe(24)
    expect(count('dots', 34, '.dot')).toBe(12)
    expect(count('bars', 96, '.bar')).toBe(44)
    expect(count('bars', 34, '.bar')).toBe(22)
    expect(count('orbit', 96, '.dot')).toBe(5)
    expect(count('ripple', 96, '.ripple')).toBe(2)
    expect(count('liquid', 96, '.blob')).toBe(3)
    expect(count('viewfinder', 96, '.corner')).toBe(4)
    expect(count('classic', 96, '.classic')).toBe(1)
  })

  it('bar_delays_and_still_profile_are_computed_at_render', () => {
    withUser(person('bars'))
    const box = render({ d: 96 })
    const bars = [...box.querySelectorAll<HTMLElement>('.bar')]
    bars.forEach((b, i) => {
      const s = Math.sin(i * 0.9)
      expect(parseFloat(b.style.getPropertyValue('--delay'))).toBeCloseTo(-((s + 1) * 0.25), 3)
      expect(parseFloat(b.style.getPropertyValue('--rest'))).toBeCloseTo(0.35 + (0.4 * (s + 1)) / 2, 3)
    })
    // 22 bars below 48px take every other holder, so their angles still go all the way round
    unmount(cmp!)
    cmp = undefined
    const small = render({ d: 34 })
    const angles = [...small.querySelectorAll<HTMLElement>('.bars .holder')].map((h) => parseFloat(/rotate\(([-\d.]+)deg\)/.exec(h.style.transform)![1]))
    expect(angles[1] - angles[0]).toBeCloseTo((360 / 44) * 2, 3)
  })

  it('going_off_fades_then_clears_lvl', () => {
    vi.useFakeTimers()
    try {
      withUser(person('bars'))
      const box = render({ speaking: true })
      box.style.setProperty('--lvl', '.9')
      props.speaking = false
      flushSync()
      // it eases out first (Phase 2 review: no snap from a level to nothing)...
      expect(box.style.getPropertyValue('--lvl')).toBe('.9')
      vi.advanceTimersByTime(400)
      // ...then a later soundboard-lit ring runs at the .5 fallback (§11.7)
      expect(box.style.getPropertyValue('--lvl')).toBe('')
      // speaking again before the fade ends keeps whatever level arrives next
      props.speaking = true
      flushSync()
      box.style.setProperty('--lvl', '.4')
      props.speaking = false
      flushSync()
      props.speaking = true
      flushSync()
      vi.advanceTimersByTime(400)
      expect(box.style.getPropertyValue('--lvl')).toBe('.4')
    } finally {
      vi.useRealTimers()
    }
  })

  it('classic_for_all_renders_classic_in_the_persons_colour', () => {
    withUser(person('bars', 'purple'))
    ringPrefs.classic = true
    const box = render({ speaking: true })
    expect(box.dataset.style).toBe('classic')
    expect(box.style.getPropertyValue('--speak')).toBe('var(--speak-purple)')
    expect(box.querySelectorAll('.bar')).toHaveLength(0)
  })

  it('the_preview_draws_its_overrides_and_ignores_the_classic_setting', () => {
    ringPrefs.classic = true
    const box = render({ place: 'preview', style: 'orbit', speaking: true })
    expect(box.dataset.style).toBe('orbit')
    expect(box.dataset.ring).toBe('on')
  })

  it('the_preview_takes_the_drafts_profile_colour', () => {
    // the saved you has no name colour; the draft (not saved yet) has one: the preview shows the draft's
    withUser(person())
    const draft: User = { ...person(), style: { font: null, colors: ['#ffc4a3'] } }
    const box = render({ place: 'preview', style: 'pulse', color: 'profile', user: draft, speaking: true })
    expect(box.style.getPropertyValue('--speak')).toBe(speakCss(draft, 'profile', nameBg('side')))
    expect(box.style.getPropertyValue('--speak')).not.toBe('var(--speak-green)')
  })

  it('only_reactive_visible_rings_watch', () => {
    const last = () => h.watching.at(-1)?.[1] ?? false
    withUser(person('bars'))
    render()
    expect(last()).toBe(true)
    motion.reducedMotion = true
    flushSync()
    expect(last()).toBe(false)
    unmount(cmp!)
    cmp = undefined
    motion.reducedMotion = false

    withUser(person('pulse'))
    h.watching.length = 0
    render()
    expect(last()).toBe(false)
    unmount(cmp!)
    cmp = undefined

    withUser(person('bars'))
    h.watching.length = 0
    render({ place: 'preview' })
    expect(last()).toBe(false)
    unmount(cmp!)
    cmp = undefined

    // the Classic setting means no reactive ring, so no levels
    withUser(person('bars'))
    ringPrefs.classic = true
    h.watching.length = 0
    render()
    expect(last()).toBe(false)
  })
})
