import type { ConnState } from './state'

/** What the top bar says while we're not connected. */
export function connLabel(c: ConnState): string {
  if (typeof c === 'object') return `Too many attempts, retrying in ${c.rate_limited}s…`
  if (c === 'too_many_devices') return 'Too many devices connected — close Pulse on another device'
  return c === 'reconnecting' ? 'Reconnecting…' : 'Connecting…'
}
