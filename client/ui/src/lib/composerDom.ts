// The message box's editable DOM, kept to one rule: the message text (what's sent, and what drafts
// keep) is the source of truth. Custom emoji tokens (`<:name:id>` / `<a:name:id>`) show as images
// that serialize back to exactly their token; everything else is plain text (newlines included,
// shown with `white-space: pre-wrap`).
import { mediaUrl, TOKEN_RE } from './emoji'

type Emoji = { id: string; name: string; hash: string | null; still_hash: string | null; animated: boolean }

/** Where each custom emoji token is in `text`. */
export function tokenSpans(text: string): { start: number; end: number; id: string }[] {
  const out: { start: number; end: number; id: string }[] = []
  for (const m of text.matchAll(new RegExp(TOKEN_RE, 'g'))) out.push({ start: m.index, end: m.index + m[0].length, id: m[3] })
  return out
}

/** Replace `el`'s content with `text`: known custom emoji as images (an unknown or deleted one stays
 * as its text). Returns the images made, for the caller to animate. */
export function renderInto(el: HTMLElement, text: string, lookup: (id: string) => Emoji | undefined): HTMLImageElement[] {
  const nodes: Node[] = []
  const imgs: HTMLImageElement[] = []
  let at = 0
  for (const t of tokenSpans(text)) {
    const e = lookup(t.id)
    if (!e?.hash) continue
    if (t.start > at) nodes.push(document.createTextNode(text.slice(at, t.start)))
    const img = document.createElement('img')
    img.src = mediaUrl(e.still_hash ?? e.hash)
    img.alt = `:${e.name}:`
    img.className = 'emoji'
    img.draggable = false
    img.dataset.token = text.slice(t.start, t.end)
    if (e.animated && e.still_hash) img.dataset.anim = e.hash
    img.setAttribute('contenteditable', 'false')
    nodes.push(img)
    imgs.push(img)
    at = t.end
  }
  if (at < text.length) nodes.push(document.createTextNode(text.slice(at)))
  // a line that ends the text needs something after it to be a line the caret can sit on
  if (text.endsWith('\n')) {
    const pad = document.createElement('br')
    pad.dataset.pad = ''
    nodes.push(pad)
  }
  el.replaceChildren(...nodes)
  return imgs
}

const BLOCK = new Set(['DIV', 'P'])

/** The message text `el` holds, however the browser arranged it while editing (it wraps lines in
 * `<div>`s and pads empty ones with `<br>`). */
export function serialize(el: Node): string {
  let out = ''
  let afterBlock = false
  const inline = (s: string) => {
    if (!s) return
    if (afterBlock) out += '\n'
    afterBlock = false
    out += s
  }
  for (const n of el.childNodes) {
    if (n.nodeType === Node.TEXT_NODE) {
      inline((n as Text).data)
    } else if (n instanceof HTMLImageElement) {
      if (n.dataset.token) inline(n.dataset.token)
    } else if (n instanceof HTMLBRElement) {
      // the last <br> of a line is the browser's placeholder (or our pad), not a newline
      if (n === el.lastChild) continue
      inline('\n')
    } else if (n instanceof HTMLElement && BLOCK.has(n.nodeName)) {
      if (out || afterBlock) out += '\n'
      afterBlock = false
      out += serialize(n)
      afterBlock = true
    } else {
      inline(serialize(n))
    }
  }
  return out
}

/** The text offset of DOM position (`node`, `offset`) inside `el`. */
export function offsetOf(el: HTMLElement, node: Node, offset: number): number {
  if (!el.contains(node)) return serialize(el).length
  const r = document.createRange()
  r.setStart(el, 0)
  r.setEnd(node, offset)
  const before = document.createElement('div')
  before.append(r.cloneContents())
  // a line break the range ends right after is a newline here, not a placeholder
  const text = serialize(before)
  const last = before.lastChild
  return last instanceof HTMLBRElement && !last.dataset.pad ? text.length + 1 : text.length
}

/** The DOM position for text offset `at` in `el` as `renderInto` left it; inside or at the end of a
 * custom emoji, just after its image. */
export function positionAt(el: HTMLElement, at: number): { node: Node; offset: number } {
  let pos = 0
  const kids = [...el.childNodes]
  for (let i = 0; i < kids.length; i++) {
    const n = kids[i]
    if (n.nodeType === Node.TEXT_NODE) {
      const len = (n as Text).data.length
      if (at <= pos + len) return { node: n, offset: at - pos }
      pos += len
    } else if (n instanceof HTMLImageElement && n.dataset.token) {
      const len = n.dataset.token.length
      if (at < pos + len || (at === pos + len && !(kids[i + 1]?.nodeType === Node.TEXT_NODE))) return { node: el, offset: i + 1 }
      pos += len
    } else if (n instanceof HTMLBRElement && n.dataset.pad !== undefined) {
      return { node: el, offset: i }
    }
  }
  const last = kids[kids.length - 1]
  if (last?.nodeType === Node.TEXT_NODE) return { node: last, offset: (last as Text).data.length }
  return { node: el, offset: kids.length }
}
