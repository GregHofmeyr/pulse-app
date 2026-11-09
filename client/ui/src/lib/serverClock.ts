// The server's time, as best this client can tell (#258): a timed pin runs out by the server's clock, so a local
// clock that runs ahead would otherwise hide a pin the server still holds (and offer Pin, which it refuses).
//
// Each server stamp that reaches us (Ready's `server_now`, a live message's `created_at`) was made no later than it
// arrived, so `stamp - local arrival` is a lower bound on how far the server's clock is ahead of ours; the largest
// of the recent ones is the best guess. Only the recent ones: a local clock set right mid-session wins out soon.

/** How many recent stamps the guess is taken from. */
const KEPT = 8

let offsets: number[] = []

/** A server stamp `iso` that arrived at local time `at`. */
export function noteServerTime(iso: string | null | undefined, at = Date.now()): void {
  const t = iso ? Date.parse(iso) : NaN
  if (Number.isNaN(t)) return
  offsets = [...offsets.slice(-(KEPT - 1)), t - at]
}

/** Start over from a (re)connect's Ready: its `server_now` (absent from older servers), arrived at `at`. */
export function resetServerClock(iso: string | null | undefined, at = Date.now()): void {
  offsets = []
  noteServerTime(iso, at)
}

/** The server's time now (ms), or the local clock's while no stamp has come. */
export function serverNow(at = Date.now()): number {
  return offsets.length ? at + Math.max(...offsets) : at
}
