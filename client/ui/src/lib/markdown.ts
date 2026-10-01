// Markdown → safe HTML. Raw HTML in messages is shown as text; output goes through a strict
// DOMPurify allow-list. This is the ONLY string the UI may pass to {@html}.
import DOMPurify from 'dompurify'
import { Marked } from 'marked'

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

export function renderMarkdown(src: string): string {
  const html = marked.parse(src, { async: false }) as string
  return purifier()
    .sanitize(html, { ALLOWED_TAGS, ALLOWED_ATTR: ['href', 'target', 'rel'], ALLOW_DATA_ATTR: false })
    .trim()
}

export const TYPING_EVERY_MS = 3000

/** Send a typing signal at most once per 3 s. */
export function typingThrottle(lastSentAt: number | null, now: number): boolean {
  return lastSentAt === null || now - lastSentAt >= TYPING_EVERY_MS
}
