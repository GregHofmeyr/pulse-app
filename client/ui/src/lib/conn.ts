import type { ConnState } from './state'

/** How long the connection is down before the top bar says so: a quick reconnect doesn't flash it. */
export const CONN_PILL_DELAY_MS = 600

/** The server turned this connection away (and we're still retrying). */
export function isRefusal(c: ConnState): boolean {
  return c === 'too_many_devices' || typeof c === 'object'
}

/** Whole seconds left of a "retrying in" wait that started at `since` (ms); never negative. */
export function secondsLeft(c: { rate_limited: number }, since: number, now: number): number {
  return Math.max(0, Math.ceil((since + c.rate_limited * 1000 - now) / 1000))
}

/** What the top bar says while we're not connected. `since`/`now` (ms) tick the 429 countdown. */
export function connLabel(c: ConnState, since = 0, now = since): string {
  if (typeof c === 'object') {
    const s = secondsLeft(c, since, now)
    return s > 0 ? `Too many attempts, retrying in ${s}s…` : 'Connecting…'
  }
  if (c === 'too_many_devices') return 'Too many devices connected — close Pulse on another device'
  return c === 'reconnecting' ? 'Reconnecting…' : 'Connecting…'
}
