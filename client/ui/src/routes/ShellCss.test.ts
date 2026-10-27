// @vitest-environment jsdom
// Layout rules jsdom can check from the shell's own compiled CSS (it applies the cascade; it
// doesn't lay out, so position/size rules are checked as computed values).
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { compile } from 'svelte/compiler'
import { beforeAll, describe, expect, it } from 'vitest'

let scope = ''
beforeAll(() => {
  const src = readFileSync(resolve(__dirname, 'Shell.svelte'), 'utf8')
  const css = compile(src, { filename: 'Shell.svelte', css: 'external' }).css!.code
  scope = /\.menu\.(svelte-[a-z0-9]+)/.exec(css)![1]
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
    const menu = el('<div class="panel-head $s"><div data-probe class="menu head-menu $s"><button class="$s">Server settings</button></div></div>')
    const s = getComputedStyle(menu)
    expect(s.top).toBe('calc(100% + 4px)')
    expect(s.bottom).toBe('auto')
  })

  it('the account menu still opens upward', () => {
    const menu = el('<div><div data-probe class="menu $s"></div></div>')
    expect(getComputedStyle(menu).bottom).toBe('calc(100% + 6px)')
  })
})
