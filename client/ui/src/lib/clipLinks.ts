// One signed play link per clip, shared by every player of it and reused until shortly before it
// expires (#268). The store's answers say they may be cached as immutable, but the HTTP cache keys
// on the whole URL, signature included: only a reused link lets a replay come from the cache.
import type { ClipUrl } from './protocol/ClipUrl'

/** A link this close to expiring is not handed out again: a clip (90 s at most) started on it must finish on it. */
export const RENEW_MARGIN_MS = 5 * 60_000

const links = new Map<string, { url: string; until: number }>()
const pending = new Map<string, Promise<string>>()
let era = 0 // bumped by forgetClipLinks, so a fetch from before a sign-out stores nothing

/** When to stop handing out `link`, on this device's clock. The link's own lifetime (`X-Amz-Expires`)
 * counted from when we asked is immune to a skewed clock; `expires_at` is the fallback. Unknown: now. */
function until(link: ClipUrl, askedAt: number): number {
  let ttl = NaN
  try {
    ttl = Number(new URL(link.url).searchParams.get('X-Amz-Expires') ?? NaN)
  } catch {
    // not a URL we can read: fall back below
  }
  if (ttl > 0) return askedAt + ttl * 1000 - RENEW_MARGIN_MS
  const at = Date.parse(link.expires_at)
  return Number.isFinite(at) ? at - RENEW_MARGIN_MS : askedAt
}

/** The play link for clip message `id`: the stored one while it has time left, else a new one from
 * `fetch`. Players asking at once share one request. `fresh` replaces the stored link (a playback
 * error that may be an expired or refused link). */
export function clipLink(id: string, fetch: (id: string) => Promise<ClipUrl>, opts: { fresh?: boolean } = {}): Promise<string> {
  if (!opts.fresh) {
    const have = links.get(id)
    if (have && Date.now() < have.until) return Promise.resolve(have.url)
    const inFlight = pending.get(id)
    if (inFlight) return inFlight
  }
  links.delete(id)
  const mine = era
  const askedAt = Date.now()
  const p = fetch(id).then(
    (link) => {
      if (mine === era) {
        links.set(id, { url: link.url, until: until(link, askedAt) })
        if (pending.get(id) === p) pending.delete(id)
      }
      return link.url
    },
    (e) => {
      if (mine === era && pending.get(id) === p) pending.delete(id)
      throw e
    },
  )
  pending.set(id, p)
  return p
}

/** Sign-out or account switch: the next account asks for its own links. */
export function forgetClipLinks(): void {
  era++
  links.clear()
  pending.clear()
}
