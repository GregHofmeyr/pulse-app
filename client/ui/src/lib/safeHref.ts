/** A link from the server or another person, fit for an `href`: an http(s) URL with a host, as given. Anything else
 *  (`javascript:`, `file:`, `data:`, a UNC path, garbage) is `undefined`, so the link renders without an `href` and
 *  goes nowhere. The app opens web links in the browser (links.rs); this keeps other schemes out of the page too. */
export function safeHref(url: string | null | undefined): string | undefined {
  if (!url) return undefined
  let u: URL
  try {
    u = new URL(url)
  } catch {
    return undefined
  }
  return (u.protocol === 'http:' || u.protocol === 'https:') && u.hostname !== '' ? url : undefined
}
