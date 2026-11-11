// Signed storage links, reused until shortly before they expire (#268): the store's answers may be cached as
// immutable, but the HTTP cache keys on the whole URL, signature included, so only a reused link lets a replay or a
// re-render come from the cache. Clip play links (clipLinks.ts) and message files (attachmentLinks.ts) share this.

/** A link this close to expiring is not handed out again: a clip (90 s at most) started on it must finish on it. */
export const RENEW_MARGIN_MS = 5 * 60_000

export type SignedLink = { url: string; expires_at: string }

/** When to stop handing out `link`, on this device's clock. The link's own lifetime (`X-Amz-Expires`)
 * counted from when we asked is immune to a skewed clock; `expires_at` is the fallback. Unknown: now. */
export function linkUntil(link: SignedLink, askedAt: number): number {
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

/** A cache of links by key. `get` hands out a stored link with time left, or joins or starts a fetch. */
export function linkCache() {
  const links = new Map<string, { url: string; until: number }>()
  const pending = new Map<string, Promise<string>>()
  let era = 0 // bumped by forget, so a fetch from before a sign-out stores nothing

  return {
    /** The link for `key`: the stored one while it has time left, else `fetch`'s. Callers asking at once share one
     *  request. `fresh` replaces the stored link (a load error that may be an expired or refused link). */
    get(key: string, fetch: () => Promise<SignedLink>, opts: { fresh?: boolean } = {}): Promise<string> {
      if (!opts.fresh) {
        const have = links.get(key)
        if (have && Date.now() < have.until) return Promise.resolve(have.url)
        const inFlight = pending.get(key)
        if (inFlight) return inFlight
      }
      links.delete(key)
      const mine = era
      const askedAt = Date.now()
      const p = fetch().then(
        (link) => {
          if (mine === era) {
            links.set(key, { url: link.url, until: linkUntil(link, askedAt) })
            if (pending.get(key) === p) pending.delete(key)
          }
          return link.url
        },
        (e) => {
          if (mine === era && pending.get(key) === p) pending.delete(key)
          throw e
        },
      )
      pending.set(key, p)
      return p
    },
    /** Store a link fetched alongside another (a message's other files). */
    put(key: string, link: SignedLink, askedAt: number) {
      links.set(key, { url: link.url, until: linkUntil(link, askedAt) })
    },
    /** Sign-out or account switch: the next account asks for its own links. */
    forget() {
      era++
      links.clear()
      pending.clear()
    },
  }
}
