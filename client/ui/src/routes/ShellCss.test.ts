// @vitest-environment jsdom
// Layout rules jsdom can check from the shell's own compiled CSS (it applies the cascade; it
// doesn't lay out, so position/size rules are checked as computed values).
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { compile } from 'svelte/compiler'
import { beforeAll, describe, expect, it } from 'vitest'

// a fixed scope class, so the probes below needn't scrape Svelte's hash
const scope = 'svelte-test'
let src = ''
beforeAll(() => {
  // a path, not new URL(…, import.meta.url): under the jsdom environment that URL isn't file:
  src = readFileSync(resolve(__dirname, 'Shell.svelte'), 'utf8')
  const css = compile(src, { filename: 'Shell.svelte', css: 'external', cssHash: () => scope }).css!.code
  const style = document.createElement('style')
  style.textContent = css
  document.head.append(style)
})

const el = (html: string) => {
  document.body.innerHTML = html.replaceAll('$s', scope)
  return document.body.firstElementChild!.querySelector<HTMLElement>('[data-probe]')!
}

describe('the server name menu', () => {
  it('drops down from the header with its own height (not squeezed between top and bottom)', () => {
    const menu = el('<div class="panel-head $s"><div data-probe class="menu head-menu menu-surface $s"><button class="$s">Server settings</button></div></div>')
    const s = getComputedStyle(menu)
    expect(s.top).toBe('calc(100% + 4px)')
    expect(s.bottom).toBe('auto')
    // its items read like the account menu's, not the bold header they sit under
    expect(getComputedStyle(menu.querySelector('button')!).fontWeight).toBe('400')
    // the probe matches what the template renders
    expect(src).toContain('class="menu head-menu menu-surface"')
  })

  it('the account menu still opens upward', () => {
    const menu = el('<div><div data-probe class="menu $s"></div></div>')
    expect(getComputedStyle(menu).bottom).toBe('calc(100% + 6px)')
  })
})

describe('the side panels in full view', () => {
  it('hidden really hides them; shown, the member list wrapper is a real box', () => {
    expect(getComputedStyle(el('<div><aside data-probe hidden class="panel side $s"></aside></div>')).display).toBe('none')
    expect(getComputedStyle(el('<div><aside data-probe class="panel side $s"></aside></div>')).display).toBe('flex')
    expect(getComputedStyle(el('<div><div data-probe hidden class="right-panel $s"></div></div>')).display).toBe('none')
    // a real box (a display:contents one is dropped from the accessibility tree, and aria-controls points at it)
    expect(getComputedStyle(el('<div><div data-probe class="right-panel $s"><aside></aside></div></div>')).display).toBe('flex')
    // nothing in it (Home): no box, so no extra gap
    expect(getComputedStyle(el('<div><div data-probe class="right-panel $s"></div></div>')).display).toBe('none')
  })
})
