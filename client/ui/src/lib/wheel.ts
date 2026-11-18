// The picker wheel's pure parts (#158): the `/wheel` command, editing a list of options, and the spin's geometry.
// The server draws every spin's winner and final angle; these only read and animate them.

import { wheel_limits } from './protocol/limits'

/** The server's `wheel_limits` (generated into protocol/limits.ts, #415). */
export const WHEEL_LIMITS = {
  minOptions: wheel_limits.MIN_OPTIONS,
  maxOptions: wheel_limits.MAX_OPTIONS,
  optionChars: wheel_limits.OPTION_CHARS,
  nameChars: wheel_limits.NAME_CHARS,
  maxWeight: wheel_limits.MAX_WEIGHT,
} as const

/** A full spin: whole turns before the landing, over this long. Every client uses the same, so all show one spin. */
export const SPIN_TURNS = 5
export const SPIN_MS = 4500
/** Reduced motion, streaming or an unfocused window: a short turn to the landing, no tick. */
export const QUIET_SPIN_MS = 400
/** A spin that reaches a client this recently is shown spinning when its line mounts; older ones mount settled. */
export const LIVE_WINDOW_MS = 1000
/** At most one tick this often, however fast the slices pass. */
export const TICK_GAP_MS = 80
/** A quiet spin's card (#270) leaves nothing behind: it shows the result this long after landing, then fades. */
export const QUIET_LINGER_MS = 6000
/** How long a quiet spin's card lasts from when it arrives: the full spin, then the result for a while. */
export const QUIET_CARD_MS = SPIN_MS + QUIET_LINGER_MS

/** A wheel's weights (#259) for `n` options: the saved ones when they fit, else every option alike. */
export function weightsFor(n: number, weights?: readonly number[] | null): number[] {
  return weights && weights.length === n && weights.some((w) => w > 0) ? weights.map((w) => Math.max(0, w)) : Array(n).fill(1)
}

/** Each slice's start and size in degrees, clockwise from 12 o'clock (#259: its weight's share of the turn). */
export function sliceSpans(n: number, weights?: readonly number[] | null): { from: number; seg: number }[] {
  const w = weightsFor(n, weights)
  const total = w.reduce((a, b) => a + b, 0)
  let before = 0
  return w.map((x) => {
    const span = { from: (360 * before) / total, seg: (360 * x) / total }
    before += x
    return span
  })
}

/** Weights that are all alike are none (an unweighted wheel), as the server stores them. */
export const cleanWeights = (weights: readonly number[]): number[] | undefined =>
  weights.length && weights.some((w) => w !== weights[0]) ? [...weights] : undefined

/** A draw on this device, for a Just me wheel's private spin (#270): the winner, in proportion to its weight
 *  (#259; uniformly without weights), and the final angle that puts the pointer 10-90 % of the way across its slice.
 *  The server's `draw_weighted` (server/src/wheels/mod.rs). */
export function drawSpin(n: number, random: () => number = Math.random, weights?: readonly number[] | null): { winner: number; angle: number } {
  const w = weightsFor(n, weights)
  const total = w.reduce((a, b) => a + b, 0)
  const pick = random() * total
  let winner = n - 1
  for (let i = 0, end = 0; i < n; i++) {
    end += w[i]
    if (pick < end) {
      winner = i
      break
    }
  }
  const { from, seg } = sliceSpans(n, w)[winner]
  const at = from + seg * (0.1 + random() * 0.8)
  return { winner, angle: (((360 - at) % 360) + 360) % 360 }
}

/** "a, b, c" (or one per line) → ["a", "b", "c"]: trimmed, blanks dropped. */
export function splitOptions(text: string): string[] {
  return text
    .split(/[,\n]/)
    .map((s) => s.trim())
    .filter(Boolean)
}

/** Add what was typed or pasted (`a, b, c` adds several) to `list`: each cut to the character limit, ones already
 *  there exactly skipped (case counts: `UPPER` and `upper` are two options, #271), and never past the option limit.
 *  `full`: some didn't fit. */
export function addOptions(list: string[], text: string): { options: string[]; full: boolean } {
  const out = [...list]
  const seen = new Set(list)
  let full = false
  for (const raw of splitOptions(text)) {
    const o = [...raw].slice(0, WHEEL_LIMITS.optionChars).join('').trim()
    if (seen.has(o)) continue
    if (out.length >= WHEEL_LIMITS.maxOptions) {
      full = true
      break
    }
    seen.add(o)
    out.push(o)
  }
  return { options: out, full }
}

/** A shuffled copy (Fisher-Yates). */
export function shuffled<T>(list: T[], random: () => number = Math.random): T[] {
  const a = [...list]
  for (let i = a.length - 1; i > 0; i--) {
    const j = Math.floor(random() * (i + 1))
    ;[a[i], a[j]] = [a[j], a[i]]
  }
  return a
}

/** The slice under the pointer at 12 o'clock when the wheel has turned `angle` degrees clockwise (slice i of n
 *  covers [i·360/n, (i+1)·360/n) clockwise from 12 o'clock): `wheel_winner_at` in protocol/src/rest.rs. */
export function winnerAt(angle: number, n: number, weights?: readonly number[] | null): number {
  if (n <= 0) return 0
  const mod = (v: number) => ((v % 360) + 360) % 360
  const at = mod(360 - mod(angle))
  if (!weights || weights.length !== n) return Math.min(n - 1, Math.floor(at / (360 / n)))
  // weighted (#259): `wheel_winner_at_weighted`
  const spans = sliceSpans(n, weights)
  const i = spans.findIndex((s) => at < s.from + s.seg)
  return i < 0 ? n - 1 : i
}

const easeOut = (t: number) => 1 - (1 - t) ** 4

/** The wheel's rotation `elapsed` ms into a spin that lands at `angle`. A full spin turns SPIN_TURNS times first;
 *  a quiet one only turns to the landing. Ends exactly on `angle` (+ whole turns). */
export function rotationAt(elapsed: number, angle: number, quiet: boolean): number {
  const duration = quiet ? QUIET_SPIN_MS : SPIN_MS
  const target = (quiet ? 0 : SPIN_TURNS * 360) + angle
  const t = Math.min(1, Math.max(0, elapsed / duration))
  return target * easeOut(t)
}

export const spinDuration = (quiet: boolean) => (quiet ? QUIET_SPIN_MS : SPIN_MS)

/** A slice label's font size (wheel radius 100) and how many characters fit along the slice. */
export function labelFit(n: number): { size: number; chars: number } {
  // as wide as the slice is at 70 % of the radius allows, 9 at most
  const size = Math.max(3.5, Math.min(9, ((2 * Math.PI * 70) / Math.max(1, n)) * 0.8))
  const chars = Math.max(3, Math.floor(64 / (size * 0.56)))
  return { size, chars }
}

/** The result view's winner (#271): its font size in px on a wheel `size` px wide. Up to 8 characters at 15 % of the
 *  wheel; longer ones shrink (they wrap onto a few lines too), never under half that. */
export function resultFontSize(text: string, size: number): number {
  const big = size * 0.15
  const chars = [...text].length
  return Math.round(Math.max(big / 2, chars <= 8 ? big : big * Math.sqrt(8 / chars)) * 10) / 10
}

/** `text` cut to `chars` characters with an ellipsis. */
export function fitLabel(text: string, chars: number): string {
  const c = [...text]
  return c.length <= chars ? text : c.slice(0, Math.max(1, chars - 1)).join('').trimEnd() + '…'
}
