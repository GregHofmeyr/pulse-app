/** Hover → prefetch a voice channel's join token, without burning the server's per-user token
 *  limit: the fetch starts only once the pointer (or focus) rests on a channel for `delayMs`, so
 *  sweeping down the list fetches nothing, and a channel prefetched within `freshMs` isn't asked
 *  for again (the core keeps that token for 2 min; this stays a little under that). Only a
 *  prefetch that succeeded counts as fresh: a failed one is asked for again on the next rest, and
 *  one still in flight isn't asked for twice. `cancel` (also on a click) drops a pending timer. */
export const HOVER_DELAY_MS = 200
export const FRESH_MS = 100_000

export function hoverPrefetch(
  fetch: (channelId: string) => Promise<unknown>,
  { delayMs = HOVER_DELAY_MS, freshMs = FRESH_MS, now = () => Date.now() } = {},
) {
  let timer: ReturnType<typeof setTimeout> | undefined
  let pending: string | null = null
  let last: { id: string; at: number } | null = null
  let inFlight: string | null = null

  function cancel() {
    if (timer !== undefined) clearTimeout(timer)
    timer = undefined
    pending = null
  }

  return {
    /** The pointer or focus arrived on `channelId`. */
    enter(channelId: string) {
      if (pending === channelId) return
      cancel()
      if (inFlight === channelId) return
      if (last && last.id === channelId && now() - last.at < freshMs) return
      pending = channelId
      timer = setTimeout(() => {
        timer = undefined
        pending = null
        inFlight = channelId
        fetch(channelId).then(
          () => {
            last = { id: channelId, at: now() }
          },
          () => {},
        ).finally(() => {
          if (inFlight === channelId) inFlight = null
        })
      }, delayMs)
    },
    /** It left `channelId` before the delay ran out: fetch nothing. */
    leave(channelId: string) {
      if (pending === channelId) cancel()
    },
    cancel,
  }
}
