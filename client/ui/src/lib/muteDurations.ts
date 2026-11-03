// How long a mute lasts: one list for every mute menu (a channel, a conversation, a server).

/** `minutes` null = until turned back on. */
export const MUTE_FOR: readonly { label: string; minutes: number | null }[] = [
  { label: 'For 15 minutes', minutes: 15 },
  { label: 'For 1 hour', minutes: 60 },
  { label: 'For 8 hours', minutes: 8 * 60 },
  { label: 'For 24 hours', minutes: 24 * 60 },
  { label: 'Until I turn it back on', minutes: null },
]

/** When a mute of `minutes` set at `now` (ms) ends, as the server takes it (null: never). */
export function muteUntil(minutes: number | null, now = Date.now()): string | null {
  return minutes === null ? null : new Date(now + minutes * 60_000).toISOString()
}
