// @vitest-environment jsdom
import DOMPurify from 'dompurify'
import { describe, expect, it, vi } from 'vitest'
import { RENDER_CACHE_MAX, renderMarkdown, renderMessage, sanitize, typingThrottle } from './markdown'
import type { CustomEmoji } from './protocol/CustomEmoji'

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

describe('renderMessage (cached)', () => {
  const msg = (id: string, content: string, edited_at: string | null = null) => ({ id, content, edited_at })

  it('renders each message once, however often it is shown', () => {
    const spy = vi.spyOn(DOMPurify, 'sanitize')
    const names = ['sam']
    const a = renderMessage(msg('01A', 'hi **@sam**'), names)
    const b = renderMessage(msg('01A', 'hi **@sam**'), names)
    expect(b).toBe(a)
    expect(spy).toHaveBeenCalledTimes(1)
    renderMessage(msg('01A', 'hi **@sam**'), ['sam']) // equal names, new array: still cached
    expect(spy).toHaveBeenCalledTimes(1)
    spy.mockRestore()
  })

  it('an edit or different mentionable names render again', () => {
    expect(renderMessage(msg('01B', 'old'))).toContain('old')
    expect(renderMessage(msg('01B', 'new', '2026-10-16T08:00:00Z'))).toContain('new')
    expect(renderMessage(msg('01C', '@sam'), [])).not.toContain('mention')
    expect(renderMessage(msg('01C', '@sam'), ['sam'])).toContain('<span class="mention">@sam</span>')
  })

  it('is bounded: the least recently shown entries are evicted', () => {
    renderMessage(msg('KEEP', 'kept'))
    for (let i = 0; i < RENDER_CACHE_MAX; i++) {
      renderMessage(msg(`F${i}`, 'x'))
      if (i % 100 === 0) renderMessage(msg('KEEP', 'kept')) // shown now and then: stays
    }
    const spy = vi.spyOn(DOMPurify, 'sanitize')
    renderMessage(msg('KEEP', 'kept'))
    expect(spy).not.toHaveBeenCalled()
    renderMessage(msg('F0', 'x'))
    expect(spy).toHaveBeenCalledTimes(1)
    spy.mockRestore()
  })
})

describe('emoji in messages', () => {
  const ce = (id: string, name: string, hash: string | null, animated = false): CustomEmoji => ({
    id, server_id: 'S1', name, hash, still_hash: null, animated, added_by: null, created_at: '2026-10-01T00:00:00Z',
  })
  const custom = { '1': ce('1', 'pog', 'abc123'), '2': ce('2', 'gone', null), '3': ce('3', 'party', 'def456', true) }
  const dom = (html: string) => {
    const host = document.createElement('div')
    host.innerHTML = html
    return host
  }

  it('custom_token_renders_img', () => {
    const imgs = dom(renderMarkdown('gg <:pog:1> and <a:party:3>', [], custom)).querySelectorAll('img')
    expect(imgs).toHaveLength(2)
    expect(imgs[0].getAttribute('src')).toBe('pulse-media://localhost/abc123')
    expect(imgs[0].getAttribute('alt')).toBe(':pog:')
    expect(imgs[0].className).toBe('emoji')
    expect(imgs[1].getAttribute('src')).toBe('pulse-media://localhost/def456')
  })

  it('unicode_renders_twemoji', () => {
    const imgs = dom(renderMarkdown('nice 👍 ❤️ 👍🏽 🇿🇦 👨‍👩‍👧')).querySelectorAll('img.emoji')
    expect(Array.from(imgs).map((i) => i.getAttribute('src'))).toEqual([
      '/emoji/1f44d.svg', '/emoji/2764.svg', '/emoji/1f44d-1f3fd.svg', '/emoji/1f1ff-1f1e6.svg', '/emoji/1f468-200d-1f469-200d-1f467.svg',
    ])
    expect(imgs[1].getAttribute('alt')).toBe('❤️')
  })

  it('plain digits, #, *, bare ©/® and unknown graphemes stay text', () => {
    expect(renderMarkdown('1 # * © ® ❤ é')).not.toContain('<img')
    expect(renderMarkdown('©️')).toContain('<img')
    expect(renderMarkdown('👍️')).toContain('src="/emoji/1f44d.svg"') // FE0F-qualified emoji-presentation still matches
  })

  it('tokens_in_code_stay_text', () => {
    for (const src of ['`<:pog:1>` `👍`', '```\n<:pog:1> 👍\n```', '\\<:pog:1>']) {
      const host = dom(renderMarkdown(src, [], custom))
      expect(host.querySelector('img'), src).toBeNull()
      expect(host.textContent).toContain('<:pog:1>')
    }
  })

  it('unknown_token_is_name_chip', () => {
    for (const src of ['<:gone:2>', '<:nope:999>']) {
      const host = dom(renderMarkdown(src, [], custom))
      expect(host.querySelector('img')).toBeNull()
      const chip = host.querySelector('span.emoji-gone')!
      expect(chip.textContent).toBe(src === '<:gone:2>' ? ':gone:' : ':nope:')
    }
  })

  it('only_emoji_is_big', () => {
    const host = dom(renderMarkdown('👍 <:pog:1>\n🎉', [], custom))
    const imgs = host.querySelectorAll('img')
    expect(imgs).toHaveLength(3)
    for (const i of imgs) expect(i.className).toBe('emoji big')
    expect(renderMarkdown('👍'.repeat(28))).not.toContain('big')
    expect(renderMarkdown('👍'.repeat(27))).toContain('emoji big')
  })

  it('text_plus_emoji_is_not_big', () => {
    for (const src of ['ok 👍', '`x` 👍', '<:gone:2> 👍', 'a **👍**']) {
      expect(renderMarkdown(src, [], custom), src).not.toContain('big')
    }
  })

  it('remote_img_src_still_stripped', () => {
    for (const src of ['<img src=https://x>', '<img src="pulse-media://localhost/x" onerror=alert(1)>']) {
      const host = dom(renderMarkdown(src, [], custom))
      expect(host.querySelector('img'), src).toBeNull()
    }
  })

  it('markdown_image_stays_alt_text', () => {
    const host = dom(renderMarkdown('![pic](https://evil.example/t.png)'))
    expect(host.querySelector('img')).toBeNull()
    expect(host.textContent).toBe('pic')
  })

  it('a hostile emoji hash cannot break out of the attribute', () => {
    const host = dom(renderMarkdown('<:pog:9>', [], { '9': ce('9', 'pog', '"><script>alert(1)</script>') }))
    expect(host.querySelector('script')).toBeNull()
    expect(host.querySelectorAll('img').length).toBeLessThanOrEqual(1)
    for (const el of host.querySelectorAll('*')) for (const a of el.attributes) expect(a.name.startsWith('on')).toBe(false)
  })

  it('the img allow-list only keeps emoji sources', () => {
    const ok = ['/emoji/1f44d.svg', 'pulse-media://localhost/abc', 'http://pulse-media.localhost/abc']
    for (const src of ok) {
      expect(dom(sanitize(`<img class="emoji" src="${src}" alt="x" draggable="false">`)).querySelector('img')?.getAttribute('src'), src).toBe(src)
    }
    const bad = [
      'https://evil.example/x.png', '//evil.example/x.png', 'javascript:alert(1)', ' javascript:alert(1)',
      'data:image/svg+xml,<svg onload=alert(1)>', 'data:image/png;base64,AAAA', 'pulse-media://evil/x',
      'pulse-media://localhost.evil/x', 'http://pulse-media.localhost.evil.example/x', 'https://pulse-media.localhost/x',
      'PULSE-MEDIA://localhost/x', 'http://x/emoji/a.svg', '', 'emoji/1f44d.svg',
    ]
    for (const src of bad) expect(dom(sanitize(`<img class="emoji" src="${src}">`)).querySelector('img'), src).toBeNull()
    expect(dom(sanitize('<img class="emoji">')).querySelector('img')).toBeNull()
    const kept = dom(sanitize('<img class="x" src="/emoji/a.svg" onerror="alert(1)" style="x">')).querySelector('img')!
    expect(kept.getAttribute('onerror')).toBeNull()
    expect(kept.getAttribute('style')).toBeNull()
    expect(kept.getAttribute('class')).toBeNull()
  })

  it('a deleted emoji re-renders a cached message as a chip', () => {
    const m = { id: 'E1', content: '<:pog:1>', edited_at: null }
    expect(renderMessage(m, [], custom)).toContain('<img')
    expect(renderMessage(m, [], { ...custom, '1': ce('1', 'pog', null) })).toContain('emoji-gone')
  })
})
