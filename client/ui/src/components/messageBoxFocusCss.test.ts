// @vitest-environment jsdom
// Focus style for the message box and the edit box (#41, owner's pick: "soft border"). Both are
// contenteditable EmojiBoxes with `outline: none`, so they don't get the app-wide ring; instead their edge
// turns a faint accent while focused. The message box is focused nearly all the time, which is why it's
// faint, not the ring. Read from each component's compiled CSS (jsdom can't focus a contenteditable).
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { compile } from 'svelte/compiler'
import { beforeAll, describe, expect, it } from 'vitest'

const sheetOf = (css: string) => {
  const style = document.createElement('style')
  style.textContent = css
  document.head.append(style)
  return style.sheet!
}
const compiled = (file: string) =>
  compile(readFileSync(resolve(__dirname, file), 'utf8'), { filename: file, css: 'external', cssHash: () => 's' }).css!.code
const rules = (sheet: CSSStyleSheet) => [...sheet.cssRules].filter((r): r is CSSStyleRule => r instanceof CSSStyleRule)
const ruleFor = (sheet: CSSStyleSheet, test: (selector: string) => boolean) => {
  const found = rules(sheet).find((r) => r.selectorText.split(',').some((s) => test(s.trim())))
  expect(found, 'a matching rule').toBeDefined()
  return found!.style
}

let app: CSSStyleSheet, composer: CSSStyleSheet, message: CSSStyleSheet
beforeAll(() => {
  app = sheetOf(readFileSync(resolve(__dirname, '../app.css'), 'utf8'))
  composer = sheetOf(compiled('Composer.svelte'))
  message = sheetOf(compiled('MessageItem.svelte'))
})

describe('message box and edit box focus', () => {
  it('has one soft focus colour: the accent, translucent', () => {
    const root = ruleFor(app, (s) => s === ':root')
    expect(root.getPropertyValue('--focus-soft').replace(/\s/g, '')).toBe('rgba(139,156,255,.45)')
  })

  it('the message box: a faint accent edge while focus is inside it, drawn inset so nothing moves', () => {
    const focused = ruleFor(composer, (s) => /^\.wrap\.s:focus-within$/.test(s))
    expect(focused.getPropertyValue('box-shadow')).toBe('inset 0 0 0 1px var(--focus-soft)')
    const rest = ruleFor(composer, (s) => s === '.wrap.s')
    expect(rest.getPropertyValue('transition')).toContain('box-shadow')
  })

  it('the edit box: the same faint edge on any focus (mouse too), not the full accent border', () => {
    const focused = ruleFor(message, (s) => /\.edit:focus\b/.test(s))
    expect(focused.getPropertyValue('border-color')).toBe('var(--focus-soft)')
    const css = compiled('MessageItem.svelte')
    expect(css).not.toMatch(/\.edit:focus-visible[^{]*\{[^}]*border-color:\s*var\(--accent\)/)
  })
})
