// Markdown → safe HTML. Raw HTML in messages is shown as text; output goes through a strict
// DOMPurify allow-list. This is the ONLY string the UI may pass to {@html}.
import DOMPurify from 'dompurify'
import { Marked, type TokenizerAndRendererExtension } from 'marked'
import { isMediaHash, isMediaUrl, mediaUrl, standard, TOKEN_RE, twemojiUrl } from './emoji'
import type { CustomEmoji } from './protocol/CustomEmoji'
import type { Message } from './protocol/Message'

const escapeHtml = (s: string) =>
  s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;').replace(/'/g, '&#39;')

// The custom emoji the current parse resolves tokens against (marked.parse is synchronous here).
let customNow: Record<string, CustomEmoji> = {}

const TOKEN_AT = new RegExp('^' + TOKEN_RE.source)
const TOKEN_ALL = new RegExp(TOKEN_RE.source, 'g')

// A markdown token rather than a pass over the HTML: marked then keeps it out of code spans/blocks
// and honours `\<` escapes, so neither can turn into an image.
const customEmoji: TokenizerAndRendererExtension = {
  name: 'customEmoji',
  level: 'inline',
  start: (src) => {
    const i = src.indexOf('<')
    return i < 0 ? undefined : i
  },
  tokenizer(src) {
    const m = TOKEN_AT.exec(src)
    if (m) return { type: 'customEmoji', raw: m[0], name: m[2], id: m[3] }
  },
  renderer: (t) => {
    const c = customNow[t.id as string]
    const name = escapeHtml(c?.name ?? (t.name as string))
    // Deleted or unknown: the name in a muted chip, never a broken image.
    if (!c?.hash) return `<span class="emoji-gone">:${name}:</span>`
    // An animated one renders its still frame; the page swaps in `data-anim` when it may play.
    // Kept out of this HTML, so the cached render never depends on focus or settings.
    if (c.still_hash && c.still_hash !== c.hash) {
      return `<img class="emoji" src="${escapeHtml(mediaUrl(c.still_hash))}" data-anim="${escapeHtml(c.hash)}" alt=":${name}:" draggable="false">`
    }
    return `<img class="emoji" src="${escapeHtml(mediaUrl(c.hash))}" alt=":${name}:" draggable="false">`
  },
}

const marked = new Marked({
  gfm: true,
  breaks: true,
  extensions: [customEmoji],
  renderer: {
    // Raw HTML in a message is text, never markup.
    html: ({ text }) => escapeHtml(text),
    // No remote images (tracking pixels, spec: no media in v1): show the alt text.
    image: ({ text }) => escapeHtml(text),
  },
})

const ALLOWED_TAGS = ['p', 'br', 'strong', 'em', 'del', 'code', 'pre', 'blockquote', 'ul', 'ol', 'li', 'a', 'img', 'span']
const ALLOWED_ATTR = ['href', 'target', 'rel', 'class', 'alt', 'src', 'draggable', 'data-anim']
const SAFE_HREF = /^(https?:|mailto:)/i
// The exact shapes we emit: a Twemoji file, or a stored media hash through the core's scheme.
// Anything else (remote, data:, other hosts, other paths) goes.
const TWEMOJI_SRC = /^\/emoji\/[0-9a-f]{1,6}(-[0-9a-f]{1,6})*\.svg$/
const safeImgSrc = (src: string) => TWEMOJI_SRC.test(src) || isMediaUrl(src)
// The only classes we emit; content can't set any other.
const CLASSES: Record<string, string[]> = { IMG: ['emoji', 'emoji big'], SPAN: ['mention', 'emoji-gone', 'emoji-gone big'] }

let hooked = false
function purifier() {
  if (!hooked) {
    DOMPurify.addHook('uponSanitizeAttribute', (node, data) => {
      if (data.attrName === 'data-anim') {
        if (node.tagName !== 'IMG' || !isMediaHash(data.attrValue)) data.keepAttr = false
        return
      }
      if (data.attrName !== 'src') return
      // DOMPurify's own URI check would drop the custom scheme; ours is stricter, so it decides.
      if (node.tagName === 'IMG' && safeImgSrc(data.attrValue)) data.forceKeepAttr = true
      else data.keepAttr = false
    })
    DOMPurify.addHook('afterSanitizeAttributes', (node) => {
      if (node.tagName === 'A') {
        const href = node.getAttribute('href') ?? ''
        if (!SAFE_HREF.test(href.trim())) node.removeAttribute('href')
        node.setAttribute('target', '_blank')
        node.setAttribute('rel', 'noopener noreferrer')
      }
      if (node.tagName === 'IMG') {
        if (!safeImgSrc(node.getAttribute('src') ?? '')) return node.remove()
      } else {
        for (const a of ['src', 'alt', 'draggable', 'data-anim']) node.removeAttribute(a)
      }
      const cls = node.getAttribute('class')
      if (cls !== null && !CLASSES[node.tagName]?.includes(cls)) node.removeAttribute('class')
    })
    hooked = true
  }
  return DOMPurify
}

/** The allow-list pass every rendered message goes through last. */
export function sanitize(html: string): string {
  return purifier().sanitize(html, { ALLOWED_TAGS, ALLOWED_ATTR, ALLOW_DATA_ATTR: false }).trim()
}

/** Markdown → safe HTML. Known `@names` (case-insensitive, outside code) become mention pills;
 * custom emoji tokens and Unicode emoji (outside code) become images. */
export function renderMarkdown(src: string, mentionNames: string[] = [], custom: Record<string, CustomEmoji> = {}): string {
  customNow = custom
  let html: string
  try {
    html = marked.parse(src, { async: false }) as string
  } finally {
    customNow = {}
  }
  // Pills and images are built from text nodes; the sanitiser still checks everything after.
  if (mentionNames.length || /[^\x00-\x7f]|class="emoji/.test(html)) {
    html = decorate(html, new Set(mentionNames.map((n) => n.toLowerCase())))
  }
  return sanitize(html)
}

/** Most rendered messages kept (a few long channels' worth); the least recently shown go first. */
export const RENDER_CACHE_MAX = 2000
const rendered = new Map<string, string>()
// (id, edited_at) -> the custom emoji ids its text renders: finding them may need a lex, which a
// cache hit must not pay for again. Same bound and LRU order as `rendered`.
const emojiRefs = new Map<string, string[]>()
const namesKeys = new WeakMap<string[], string>()

/** `renderMarkdown` for a message, cached: a re-render (someone typing, presence, a channel
 * revisit) reuses the HTML instead of re-running marked + DOMPurify. An edit changes `edited_at`,
 * and a different set of mentionable names is a different entry, so neither can show stale HTML.
 * So is a change to a custom emoji the message uses (a delete turns its image into a chip). */
export function renderMessage(
  m: Pick<Message, 'id' | 'edited_at' | 'content'>,
  mentionNames: string[] = [],
  customEmoji: Record<string, CustomEmoji> = {},
): string {
  let names = namesKeys.get(mentionNames)
  if (names === undefined) {
    names = mentionNames.join('\u0000')
    namesKeys.set(mentionNames, names)
  }
  const key = `${m.id}\u0001${m.edited_at ?? ''}\u0001${names}\u0001${emojiKey(m, customEmoji)}`
  const hit = rendered.get(key)
  if (hit !== undefined) {
    rendered.delete(key) // re-insert: Map order is the LRU order
    rendered.set(key, hit)
    return hit
  }
  const html = renderMarkdown(m.content, mentionNames, customEmoji)
  rendered.set(key, html)
  if (rendered.size > RENDER_CACHE_MAX) rendered.delete(rendered.keys().next().value!)
  return html
}

/** The state of each custom emoji a message's text renders, for its cache key. */
function emojiKey(m: Pick<Message, 'id' | 'edited_at' | 'content'>, custom: Record<string, CustomEmoji>): string {
  if (!m.content.includes('<')) return ''
  const v = `${m.id}\u0001${m.edited_at ?? ''}`
  let ids = emojiRefs.get(v)
  if (ids) emojiRefs.delete(v) // re-insert: Map order is the LRU order
  else ids = renderedEmojiIds(m.content)
  emojiRefs.set(v, ids)
  if (emojiRefs.size > RENDER_CACHE_MAX) emojiRefs.delete(emojiRefs.keys().next().value!)
  let k = ''
  for (const id of ids) {
    const c = custom[id]
    k += `${id}=${c?.hash ?? ''}:${c?.name ?? ''};`
  }
  return k
}

/** Tokens inside code or escaped stay text, so they don't count: without a backtick or backslash
 * none can be, and the cheap scan is exact enough (an indented code block only over-invalidates);
 * otherwise marked decides. */
function renderedEmojiIds(content: string): string[] {
  if (!/[`\\]/.test(content)) return Array.from(content.matchAll(TOKEN_ALL), (t) => t[3])
  const ids: string[] = []
  marked.walkTokens(marked.lexer(content), (t) => {
    if (t.type === 'customEmoji') ids.push(t.id as string)
  })
  return ids
}

const MENTION = /(^|[\s(])@([A-Za-z0-9_.-]{1,32})/g

// Every standard emoji as typed (base and skin tones) → the form the data stores.
const UNICODE = new Set<string>()
for (const e of standard) {
  UNICODE.add(e.u)
  for (const v of e.s ?? []) UNICODE.add(v)
}
let graphemes: Intl.Segmenter | undefined

/** The data's form of a grapheme if it's a known emoji. Emoji-presentation ones are stored without
 * FE0F but may arrive with it; text-default ones (©, digits, #) only count with their FE0F. */
function knownEmoji(g: string): string | undefined {
  if (UNICODE.has(g)) return g
  if (g.includes('\uFE0F')) {
    const bare = g.replace(/\uFE0F/g, '')
    if (UNICODE.has(bare)) return bare
  }
}

/** Unicode emoji in `text` as Twemoji images, the rest as strings. */
function emojify(text: string): (string | Node)[] {
  if (!/[^\x00-\x7f]/.test(text)) return [text]
  graphemes ??= new Intl.Segmenter(undefined, { granularity: 'grapheme' })
  const out: (string | Node)[] = []
  let buf = ''
  for (const { segment } of graphemes.segment(text)) {
    const u = knownEmoji(segment)
    if (!u) {
      buf += segment
      continue
    }
    if (buf) out.push(buf)
    buf = ''
    const img = document.createElement('img')
    img.className = 'emoji'
    img.src = twemojiUrl(u)
    img.alt = segment
    img.draggable = false
    out.push(img)
  }
  if (buf) out.push(buf)
  return out
}

/** Known @names as pills, the rest as strings. */
function mentionPills(value: string, names: Set<string>): (string | Node)[] {
  if (!names.size) return [value]
  const out: (string | Node)[] = []
  let last = 0
  for (const m of value.matchAll(MENTION)) {
    const name = m[2].replace(/\.+$/, '')
    if (!names.has(name.toLowerCase())) continue
    const at = (m.index ?? 0) + m[1].length
    out.push(value.slice(last, at))
    const pill = document.createElement('span')
    pill.className = 'mention'
    pill.textContent = '@' + name
    out.push(pill)
    last = at + 1 + name.length
  }
  out.push(value.slice(last))
  return out
}

/** A message this many emoji or fewer, and nothing else but whitespace, shows them large. */
export const BIG_EMOJI_MAX = 27

/** Mention pills and Unicode emoji in text nodes (never inside code/pre), then the big-emoji check. */
function decorate(html: string, names: Set<string>): string {
  const tpl = document.createElement('template')
  tpl.innerHTML = html
  const walker = document.createTreeWalker(tpl.content, NodeFilter.SHOW_TEXT)
  const texts: Text[] = []
  while (walker.nextNode()) {
    const t = walker.currentNode as Text
    if (!t.parentElement?.closest('code, pre')) texts.push(t)
  }
  for (const t of texts) {
    const parts = mentionPills(t.data, names).flatMap((p) => (typeof p === 'string' ? emojify(p) : [p]))
    if (parts.length === 1 && parts[0] === t.data) continue
    t.replaceWith(...parts.filter((p) => p !== ''))
  }
  // A deleted custom emoji's chip is still an emoji: an old all-emoji message stays big.
  const emoji = tpl.content.querySelectorAll('img.emoji, span.emoji-gone')
  const onlyEmoji =
    emoji.length > 0 &&
    emoji.length <= BIG_EMOJI_MAX &&
    textOutsideChips(tpl.content).trim() === '' &&
    Array.from(tpl.content.querySelectorAll('*')).every((el) => el.tagName === 'P' || el.tagName === 'BR' || el.matches('img.emoji, span.emoji-gone'))
  if (onlyEmoji) for (const e of emoji) e.classList.add('big')
  return tpl.innerHTML
}

function textOutsideChips(root: Node): string {
  let out = ''
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT)
  while (walker.nextNode()) if (!walker.currentNode.parentElement?.closest('.emoji-gone')) out += walker.currentNode.nodeValue
  return out
}

export const TYPING_EVERY_MS = 3000

/** Send a typing signal at most once per 3 s. */
export function typingThrottle(lastSentAt: number | null, now: number): boolean {
  return lastSentAt === null || now - lastSentAt >= TYPING_EVERY_MS
}
