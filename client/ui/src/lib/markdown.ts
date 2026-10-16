// Markdown → safe HTML. Raw HTML in messages is shown as text; output goes through a strict
// DOMPurify allow-list. This is the ONLY string the UI may pass to {@html}.
import DOMPurify from 'dompurify'
import { Marked } from 'marked'
import type { Message } from './protocol/Message'

const escapeHtml = (s: string) =>
  s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;').replace(/'/g, '&#39;')

const marked = new Marked({
  gfm: true,
  breaks: true,
  renderer: {
    // Raw HTML in a message is text, never markup.
    html: ({ text }) => escapeHtml(text),
    // No remote images (tracking pixels, spec: no media in v1): show the alt text.
    image: ({ text }) => escapeHtml(text),
  },
})

const ALLOWED_TAGS = ['p', 'br', 'strong', 'em', 'del', 'code', 'pre', 'blockquote', 'ul', 'ol', 'li', 'a']
const SAFE_HREF = /^(https?:|mailto:)/i

let hooked = false
function purifier() {
  if (!hooked) {
    DOMPurify.addHook('afterSanitizeAttributes', (node) => {
      if (node.tagName === 'A') {
        const href = node.getAttribute('href') ?? ''
        if (!SAFE_HREF.test(href.trim())) node.removeAttribute('href')
        node.setAttribute('target', '_blank')
        node.setAttribute('rel', 'noopener noreferrer')
      }
    })
    hooked = true
  }
  return DOMPurify
}

/** Markdown → safe HTML. Known `@names` (case-insensitive, outside code) become mention pills. */
export function renderMarkdown(src: string, mentionNames: string[] = []): string {
  const html = marked.parse(src, { async: false }) as string
  const sanitized = purifier()
    .sanitize(html, { ALLOWED_TAGS, ALLOWED_ATTR: ['href', 'target', 'rel'], ALLOW_DATA_ATTR: false })
    .trim()
  if (!mentionNames.length) return sanitized
  return withMentionPills(sanitized, new Set(mentionNames.map((n) => n.toLowerCase())))
}

/** Most rendered messages kept (a few long channels' worth); the least recently shown go first. */
export const RENDER_CACHE_MAX = 2000
const rendered = new Map<string, string>()
const namesKeys = new WeakMap<string[], string>()

/** `renderMarkdown` for a message, cached: a re-render (someone typing, presence, a channel
 * revisit) reuses the HTML instead of re-running marked + DOMPurify. An edit changes `edited_at`,
 * and a different set of mentionable names is a different entry, so neither can show stale HTML. */
export function renderMessage(m: Pick<Message, 'id' | 'edited_at' | 'content'>, mentionNames: string[] = []): string {
  let names = namesKeys.get(mentionNames)
  if (names === undefined) {
    names = mentionNames.join('\u0000')
    namesKeys.set(mentionNames, names)
  }
  const key = `${m.id}\u0001${m.edited_at ?? ''}\u0001${names}`
  const hit = rendered.get(key)
  if (hit !== undefined) {
    rendered.delete(key) // re-insert: Map order is the LRU order
    rendered.set(key, hit)
    return hit
  }
  const html = renderMarkdown(m.content, mentionNames)
  rendered.set(key, html)
  if (rendered.size > RENDER_CACHE_MAX) rendered.delete(rendered.keys().next().value!)
  return html
}

const MENTION = /(^|[\s(])@([A-Za-z0-9_.-]{1,32})/g

/** Wrap known @names in text nodes (never inside code/pre). Pills are built from text, after sanitising. */
function withMentionPills(html: string, names: Set<string>): string {
  const tpl = document.createElement('template')
  tpl.innerHTML = html
  const walker = document.createTreeWalker(tpl.content, NodeFilter.SHOW_TEXT)
  const texts: Text[] = []
  while (walker.nextNode()) {
    const t = walker.currentNode as Text
    if (!t.parentElement?.closest('code, pre')) texts.push(t)
  }
  for (const t of texts) {
    const value = t.data
    let last = 0
    const frag = document.createDocumentFragment()
    for (const m of value.matchAll(MENTION)) {
      const name = m[2].replace(/\.+$/, '')
      if (!names.has(name.toLowerCase())) continue
      const at = (m.index ?? 0) + m[1].length
      frag.append(value.slice(last, at))
      const pill = document.createElement('span')
      pill.className = 'mention'
      pill.textContent = '@' + name
      frag.append(pill)
      last = at + 1 + name.length
    }
    if (last === 0) continue
    frag.append(value.slice(last))
    t.replaceWith(frag)
  }
  return tpl.innerHTML
}

export const TYPING_EVERY_MS = 3000

/** Send a typing signal at most once per 3 s. */
export function typingThrottle(lastSentAt: number | null, now: number): boolean {
  return lastSentAt === null || now - lastSentAt >= TYPING_EVERY_MS
}
