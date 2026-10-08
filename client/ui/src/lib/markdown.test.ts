// @vitest-environment jsdom
import { describe, expect, it } from 'vitest'
import { renderMarkdown, typingThrottle } from './markdown'

const XSS = [
  '<script>alert(1)</script>',
  '<img src=x onerror=alert(1)>',
  '[click](javascript:alert(1))',
  '<a href="data:text/html,<script>alert(1)</script>">x</a>',
  '<svg onload=alert(1)>',
  '**bold** <iframe src="https://evil.example"></iframe>',
  '[x](JaVaScRiPt:alert(1))',
  '<details open ontoggle=alert(1)>',
  '![img](https://evil.example/track.png)',
]

describe('renderMarkdown', () => {
  // Escaped text like "&lt;img onerror…&gt;" is fine: it displays as text. What must never happen is
  // anything executable in the resulting DOM.
  it.each(XSS)('neutralises %s', (src) => {
    const host = document.createElement('div')
    host.innerHTML = renderMarkdown(src)
    const all = Array.from(host.querySelectorAll('*'))
    const allowed = new Set(['P', 'BR', 'STRONG', 'EM', 'DEL', 'CODE', 'PRE', 'BLOCKQUOTE', 'UL', 'OL', 'LI', 'A'])
    for (const el of all) {
      expect(allowed.has(el.tagName), `unexpected <${el.tagName}>`).toBe(true)
      for (const attr of Array.from(el.attributes)) {
        expect(attr.name.startsWith('on'), `handler ${attr.name}`).toBe(false)
        if (attr.name === 'href') expect(attr.value).toMatch(/^(https?:|mailto:)/i)
      }
    }
  })

  it('keeps basic formatting', () => {
    expect(renderMarkdown('**bold** _it_ ~~gone~~ `code`')).toBe(
      '<p><strong>bold</strong> <em>it</em> <del>gone</del> <code>code</code></p>',
    )
  })

  it('links are safe and open externally', () => {
    const out = renderMarkdown('see https://example.com/x')
    expect(out).toContain('href="https://example.com/x"')
    expect(out).toContain('rel="noopener noreferrer"')
    expect(out).toContain('target="_blank"')
  })

  it('escapes raw html as text', () => {
    expect(renderMarkdown('a <b>b</b>')).not.toContain('<b>')
  })
})

describe('typingThrottle', () => {
  it('allows at most one typing signal per 3 s', () => {
    expect(typingThrottle(null, 1000)).toBe(true)
    expect(typingThrottle(1000, 3999)).toBe(false)
    expect(typingThrottle(1000, 4000)).toBe(true)
  })
})

describe('mentions', () => {
  it('wraps known @mentions in a pill, not in code, not unknown names', () => {
    const html = renderMarkdown('hi @Sam and @nobody `@sam`', ['sam'])
    expect(html).toContain('<span class="mention">@Sam</span>')
    expect(html).not.toContain('<span class="mention">@nobody')
    expect(html).toContain('<code>@sam</code>')
  })
  it('mention markup cannot be injected by content', () => {
    const html = renderMarkdown('<span class="mention">fake</span>', [])
    expect(html).not.toContain('<span class="mention">')
  })
})
