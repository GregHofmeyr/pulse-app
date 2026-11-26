// Speaking rings (#651): the viewer's "Show everyone's rings as Classic" setting (this device only) and the cap on
// how many rings animate at once (§8.3): the first eight people to start speaking animate; the rest show still until
// one of them stops. An animating ring never drops to still because someone new started.
import { rings } from './soundRings.svelte'
import { voice } from './voice.svelte'

const CLASSIC_KEY = 'pulse.classicRings'

function savedClassic(): boolean {
  try {
    return localStorage.getItem(CLASSIC_KEY) === 'true'
  } catch {
    return false
  }
}

/** `classic`: every ring renders as Classic (colours kept). Lighter on slower PCs, calmer if motion distracts. */
export const ringPrefs = $state({ classic: savedClassic() })

export function setClassicRings(on: boolean): void {
  ringPrefs.classic = on
  try {
    localStorage.setItem(CLASSIC_KEY, String(on))
  } catch {
    /* ignore */
  }
}

export const RING_CAP = 8

/** The start order: `prev` minus whoever is no longer lit, then the newly lit in sorted order (several can start in
 * one speaking event; sorting keeps the order the same on every client). */
export function nextOrder(prev: string[], lit: Set<string>): string[] {
  const kept = prev.filter((id) => lit.has(id))
  const known = new Set(kept)
  const added = [...lit].filter((id) => !known.has(id)).sort()
  return [...kept, ...added]
}

// The order is built from the lit set (voice ∪ soundboard), never from which tiles are mounted, so a tile that
// mounts mid-speech ranks by when its person started (Review Focus 1).
let prevOrder: string[] = []
const order = $derived.by(() => {
  const lit = new Set<string>(voice.speaking)
  for (const id of rings.users) lit.add(id)
  prevOrder = nextOrder(prevOrder, lit)
  return prevOrder
})
const rank = $derived(new Map(order.map((id, i) => [id, i])))

/** Whether `id`'s lit ring may animate: among the first RING_CAP to start of those lit now. */
export function isAnimating(id: string): boolean {
  const r = rank.get(id)
  return r !== undefined && r < RING_CAP
}
