// Speaking rings (#651 §5.3–5.4): per-speaker loudness from the core (`speaking_levels`, ~14 Hz) written straight
// onto each ring element as `--lvl`. Never through $state: a reactive write 14 times a second would re-render every
// tile. The CSS reads `--lvl` only inside `scale:`, whose short transition smooths the steps on the compositor.
//
// Watching: the core sends levels only while `watch_voice_rings` is on, a flag (not a counter) that it resets when
// the page reloads. Each visible reactive ring counts here; the core hears only the 0 ↔ 1 changes, and nothing while
// the window is hidden.
import { invoke } from '@tauri-apps/api/core'
import { rings } from './soundRings.svelte'

const byUser = new Map<string, Set<HTMLElement>>()
const userOf = new WeakMap<HTMLElement, string>()
const watchers = new Set<HTMLElement>()
let asked = false
let listening = false

function visible(): boolean {
  return typeof document === 'undefined' || document.visibilityState !== 'hidden'
}

function sync(): void {
  const want = watchers.size > 0 && visible()
  if (want === asked) return
  asked = want
  void invoke('watch_voice_rings', { on: want }).catch(() => {})
}

function listen(): void {
  if (listening || typeof document === 'undefined') return
  listening = true
  document.addEventListener('visibilitychange', sync)
}

/** A ring element for `userId` (the SpeakingRing root's `{@attach}`). Returns the cleanup, which also drops its watch. */
export function register(userId: string, el: HTMLElement): () => void {
  listen()
  let set = byUser.get(userId)
  if (!set) byUser.set(userId, (set = new Set()))
  set.add(el)
  userOf.set(el, userId)
  return () => {
    const s = byUser.get(userId)
    s?.delete(el)
    if (s && !s.size) byUser.delete(userId)
    userOf.delete(el)
    buckets.delete(el)
    setWatching(el, false)
  }
}

/** Whether this element's ring wants levels now (on screen and a reactive effective style). Counted per element. */
export function setWatching(el: HTMLElement, on: boolean): void {
  listen()
  if (on) watchers.add(el)
  else watchers.delete(el)
  sync()
}

/** Wave of dots runs faster when loud (§4.10): three speeds, and a new one must hold 250 ms before it takes over. */
export type Rate = 0.75 | 1 | 1.5
export type Bucket = { rate: Rate; pending: Rate | null; since: number }

const HOLD_MS = 250

function bucketOf(lvl: number): Rate {
  return lvl < 0.35 ? 0.75 : lvl < 0.7 ? 1 : 1.5
}

export function nextBucket(b: Bucket, lvl: number, now: number): Bucket {
  const want = bucketOf(lvl)
  if (want === b.rate) return b.pending === null ? b : { ...b, pending: null }
  if (b.pending !== want) return { rate: b.rate, pending: want, since: now }
  if (now - b.since >= HOLD_MS) return { rate: want, pending: null, since: now }
  return b
}

const buckets = new WeakMap<HTMLElement, Bucket>()

/** The ring left `on`: its CSS animations restart at 1x when it next lights, so its speed bucket starts over too. */
export function resetSpeed(el: HTMLElement): void {
  buckets.delete(el)
}

function speed(el: HTMLElement, lvl: number, now: number): void {
  // only a moving Wave of dots: a still or off one has no animations to speed up, and restarts at 1x
  if (el.dataset.style !== 'dots' || el.dataset.ring !== 'on') return
  const prev = buckets.get(el) ?? { rate: 1, pending: null, since: now }
  const next = nextBucket(prev, lvl, now)
  buckets.set(el, next)
  // updatePlaybackRate changes speed without the phase jump a new animation-duration would cause
  if (next.rate !== prev.rate && typeof el.getAnimations === 'function') {
    for (const a of el.getAnimations({ subtree: true })) a.updatePlaybackRate(next.rate)
  }
}

// Ids in the last frame and this one: two sets swapped and cleared, so a frame (14 a second) allocates nothing.
let lastIds = new Set<string>()
let theseIds = new Set<string>()
const NONE: readonly HTMLElement[] = []

/** Eases a ring whose voice level stopped: to 0 with the scale transition, or, while their soundboard sound still lights
 * it, back to the calm .5 fallback (removing --lvl transitions just the same). */
function ease(id: string, el: HTMLElement): void {
  if (rings.users.has(id)) el.style.removeProperty('--lvl')
  else el.style.setProperty('--lvl', '0')
}

/** One `speaking_levels` frame. Someone missing from it (they stopped) eases down rather than snapping; `{}` (the last
 * speaker stopped) eases every ring that has a level. A ring that goes off removes its `--lvl` itself once faded. */
export function apply(levels: Record<string, number>): void {
  const now = Date.now()
  theseIds.clear()
  for (const id in levels) theseIds.add(id)
  if (!theseIds.size) {
    // a ring that never had a level (soundboard-lit) keeps its .5 fallback
    for (const [id, set] of byUser) for (const el of set) if (el.style.getPropertyValue('--lvl')) ease(id, el)
  } else {
    for (const id of lastIds) {
      if (theseIds.has(id)) continue
      for (const el of byUser.get(id) ?? NONE) ease(id, el)
    }
    for (const id of theseIds) {
      const v = levels[id]
      for (const el of byUser.get(id) ?? NONE) {
        el.style.setProperty('--lvl', String(v))
        speed(el, v, now)
      }
    }
  }
  const swap = lastIds
  lastIds = theseIds
  theseIds = swap
}
