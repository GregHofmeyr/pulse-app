// @vitest-environment jsdom
// The full view's bar, from its own compiled CSS (jsdom applies the cascade but doesn't lay out).
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { compile } from 'svelte/compiler'
import { beforeAll, describe, expect, it } from 'vitest'

const scope = 'svelte-test'
let src = ''
beforeAll(() => {
  src = readFileSync(resolve(__dirname, 'FullView.svelte'), 'utf8')
  const css = compile(src, { filename: 'FullView.svelte', css: 'external', cssHash: () => scope }).css!.code
  const style = document.createElement('style')
  style.textContent = css
  document.head.append(style)
})

const el = (html: string) => {
  document.body.innerHTML = html.replaceAll('$s', scope)
  return document.body.querySelector<HTMLElement>('[data-probe]')!
}

describe('the full view bar', () => {
  it('wraps what sits between the corner toggles (both side panels open leave little room), never the toggles', () => {
    const controls = el('<div class="bar $s"><div data-probe class="controls $s"></div></div>')
    expect(getComputedStyle(controls).flexWrap).toBe('wrap')
    expect(getComputedStyle(controls).minWidth).toBe('0px')
    const bar = el('<div data-probe class="bar $s"></div>')
    // one row is the height it always was; more rows grow it
    expect(getComputedStyle(bar).minHeight).toBe('52px')
    expect(getComputedStyle(bar).alignItems).toBe('flex-start')
    for (const sel of ['back', 'stop', 'panel-toggle']) {
      const s = getComputedStyle(el(`<div class="bar $s"><button data-probe class="${sel} $s"></button></div>`))
      expect(s.flexShrink, sel).toBe('0')
    }
    expect(getComputedStyle(el('<button data-probe class="stop $s"></button>')).whiteSpace).toBe('nowrap')
  })

  // #63
  it('full screen covers the whole window, over the app but under its dialogs (z 20 and up)', () => {
    const s = getComputedStyle(el('<div data-probe class="full-view screen $s"></div>'))
    expect(s.position).toBe('fixed')
    expect(s.zIndex).toBe('19')
    expect(getComputedStyle(el('<div data-probe class="full-view $s"></div>')).position).not.toBe('fixed')
  })

  it("the chat and full screen buttons are the corner toggles' size and look, and the strip is a panel", () => {
    const tool = getComputedStyle(el('<div class="bar $s"><button data-probe class="tool $s"></button></div>'))
    expect([tool.width, tool.height, tool.flexShrink]).toEqual(['32px', '32px', '0'])
    const on = getComputedStyle(el('<button data-probe class="tool $s" aria-pressed="true"></button>'))
    const toggleOn = getComputedStyle(el('<button data-probe class="panel-toggle $s" aria-pressed="true"></button>'))
    expect(on.backgroundColor).toBe(toggleOn.backgroundColor)
    const strip = getComputedStyle(el('<aside data-probe class="side-chat $s"></aside>'))
    expect(strip.flexShrink).toBe('0')
    expect(src).toMatch(/\.side-chat \{[^}]*background: var\(--bg-1\)[^}]*border-left: 1px solid var\(--bg-3\)/)
  })

  it('a pressed toggle looks pressed, and still answers a hover', () => {
    const on = getComputedStyle(el('<button data-probe class="panel-toggle $s" aria-pressed="true"></button>'))
    const off = getComputedStyle(el('<button data-probe class="panel-toggle $s" aria-pressed="false"></button>'))
    expect(on.color).not.toBe(off.color)
    // jsdom can't hover: the pressed hover rule exists and changes the pressed background
    expect(src).toMatch(/\.panel-toggle\[aria-pressed='true'\]:hover, \.tool\[aria-pressed='true'\]:hover \{ background: var\(--accent-strong\)/)
  })
})
