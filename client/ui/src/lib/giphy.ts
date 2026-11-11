// GIFs (#58): GIPHY links in messages, and GIPHY's media links for a GIF id. Nothing GIPHY serves is stored (its
// terms): a message or favourite keeps the id, title and size, and the GIF is drawn from GIPHY's own media links
// for that id (the ones its "copy GIF link" gives), so a GIF sent from the picker and a pasted link look the same.
// `gifIdOf` reads the same link forms as the server's `gifs::gif_id`.
import type { Message } from './protocol/Message'

/** A GIF to draw in chat: its size is unknown for a pasted link. */
export type ShownGif = { id: string; title: string; width: number | null; height: number | null }

/** Most GIFs one message shows (as link previews). */
export const MAX_GIFS = 3

const ID = /^[A-Za-z0-9]{1,64}$/

const mediaHost = (h: string) => h === 'i.giphy.com' || /^media\d*\.giphy\.com$/.test(h)

/** The GIF a GIPHY link is to (its page, embed page or a media file), else null; `link` is one link alone. */
export function gifIdOf(link: string): string | null {
  if (!link || /\s/.test(link)) return null
  let u: URL
  try {
    u = new URL(link)
  } catch {
    return null
  }
  if (u.protocol !== 'https:' && u.protocol !== 'http:') return null
  const host = u.hostname.toLowerCase()
  const segs = u.pathname.split('/').filter(Boolean)
  let id: string | undefined
  if (host === 'giphy.com' || host === 'www.giphy.com') {
    if (segs[0] === 'gifs' && segs.length >= 2) id = segs[1].split('-').pop()
    else if (segs[0] === 'embed' && segs.length >= 2) id = segs[1]
  } else if (mediaHost(host) && segs[0] === 'media') {
    if (segs.length === 4 && segs[1].startsWith('v1.')) id = segs[2]
    else if (segs.length === 3) id = segs[1]
  } else if (host === 'i.giphy.com' && segs.length === 1) {
    const dot = segs[0].indexOf('.')
    id = dot > 0 ? segs[0].slice(0, dot) : undefined
  }
  return id && ID.test(id) ? id : null
}

/** A GIF page link's words ("happy-dance-<id>" → "happy dance"); empty for any other link. */
function slugTitle(link: string): string {
  try {
    const u = new URL(link)
    const segs = u.pathname.split('/').filter(Boolean)
    if (segs[0] !== 'gifs' || !segs[1]?.includes('-')) return ''
    return decodeURIComponent(segs[1]).split('-').slice(0, -1).join(' ').trim()
  } catch {
    return ''
  }
}

/** The links in `content` that get a preview: not in code, nor wrapped as `<https://…>`; trailing punctuation off. */
function links(content: string): string[] {
  const text = content.replace(/```[\s\S]*?```/g, ' ').replace(/`[^`\n]*`/g, ' ')
  const out: string[] = []
  for (const word of text.split(/\s+/)) {
    if (!/^https?:\/\//i.test(word)) continue
    out.push(word.replace(/[.,!?;:'")\]]+$/, ''))
  }
  return out
}

/** The GIFs a message shows: a picked GIF's own, else its GIPHY links' (up to [`MAX_GIFS`], each once). */
export function gifsOf(m: Message): ShownGif[] {
  if (m.deleted) return []
  if (m.gif) return [{ id: m.gif.id, title: m.gif.title, width: m.gif.width, height: m.gif.height }]
  const out: ShownGif[] = []
  for (const link of links(m.content)) {
    const id = gifIdOf(link)
    if (!id || out.some((g) => g.id === id)) continue
    out.push({ id, title: slugTitle(link), width: null, height: null })
    if (out.length === MAX_GIFS) break
  }
  return out
}

/** The message is nothing but one GIPHY link: chat shows only the GIF, previews say "GIF". */
export const gifOnly = (m: Message): boolean => !m.deleted && gifIdOf(m.content.trim()) !== null

/** GIPHY's fixed-height (200 px) rendition of a GIF and its first frame: what chat draws. */
export const gifMedia = (id: string) => ({ anim: `https://media.giphy.com/media/${id}/200.webp`, still: `https://media.giphy.com/media/${id}/200_s.gif` })

/** GIPHY's fixed-width (200 px) rendition and its first frame: the picker's Favourites grid. */
export const gridMedia = (id: string) => ({ anim: `https://media.giphy.com/media/${id}/200w.webp`, still: `https://media.giphy.com/media/${id}/200w_s.gif` })

/** How big a GIF shows in chat: 200 px high, at most 300 px wide, by its shape; null when its shape is unknown. */
export function chatSize(g: { width: number | null; height: number | null }): { width: number; height: number } | null {
  if (!g.width || !g.height) return null
  const exact = (200 * g.width) / g.height
  if (exact <= 300) return { width: Math.round(exact), height: 200 }
  return { width: 300, height: Math.round((300 * g.height) / g.width) }
}

/** The link a picked GIF is sent as: GIPHY's page for it, or the plain page link when GIPHY gave none for it. */
export const gifLink = (r: { id: string; url: string }): string => (gifIdOf(r.url) === r.id ? r.url : `https://giphy.com/gifs/${r.id}`)

/** Items into two columns, each to the shorter one so far, keeping their order down the columns (GIPHY's grid). */
export function masonry<T>(items: T[], height: (item: T) => number): [T[], T[]] {
  const cols: [T[], T[]] = [[], []]
  const heights = [0, 0]
  for (const item of items) {
    const c = heights[1] < heights[0] ? 1 : 0
    cols[c].push(item)
    heights[c] += height(item)
  }
  return cols
}
