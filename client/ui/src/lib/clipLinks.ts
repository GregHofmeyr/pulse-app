// One signed play link per clip, shared by every player of it and reused until shortly before it
// expires (#268). The store's answers say they may be cached as immutable, but the HTTP cache keys
// on the whole URL, signature included: only a reused link lets a replay come from the cache.
import type { ClipUrl } from './protocol/ClipUrl'
import { linkCache } from './linkCache'

export { RENEW_MARGIN_MS } from './linkCache'

const cache = linkCache()

/** The play link for clip message `id`: the stored one while it has time left, else a new one from
 * `fetch`. Players asking at once share one request. `fresh` replaces the stored link (a playback
 * error that may be an expired or refused link). */
export function clipLink(id: string, fetch: (id: string) => Promise<ClipUrl>, opts: { fresh?: boolean } = {}): Promise<string> {
  return cache.get(id, () => fetch(id), opts)
}

/** Sign-out or account switch: the next account asks for its own links. */
export function forgetClipLinks(): void {
  cache.forget()
}
