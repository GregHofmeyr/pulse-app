// @vitest-environment jsdom
// One scrollbar look for the whole app, from app.css: every scrollable area (lists, the message list,
// settings pages, menus, the emoji picker, modals, the message box, code blocks) gets it, so no
// component styles its own.
import { readdirSync, readFileSync } from 'node:fs'
import { join, resolve } from 'node:path'
import { beforeAll, describe, expect, it } from 'vitest'

let sheet: CSSStyleSheet
beforeAll(() => {
  // a path, not new URL(…, import.meta.url): under the jsdom environment that URL isn't file:
  const style = document.createElement('style')
  style.textContent = readFileSync(resolve(__dirname, 'app.css'), 'utf8')
  document.head.append(style)
  sheet = style.sheet!
})

const styleRules = (rules: CSSRuleList) => [...rules].filter((r): r is CSSStyleRule => r instanceof CSSStyleRule)
const rule = (selector: string) => {
  const found = styleRules(sheet.cssRules).find((r) => r.selectorText.split(',').map((s) => s.trim()).includes(selector))
  expect(found, `a rule for ${selector}`).toBeDefined()
  return found!.style
}

describe('the global scrollbar', () => {
  it('has its colours as tokens: a translucent thumb (it reads on every surface), brighter on hover', () => {
    const root = rule(':root')
    expect(root.getPropertyValue('--scrollbar-thumb')).toMatch(/var\(--text-3\)/)
    expect(root.getPropertyValue('--scrollbar-thumb-hover')).toMatch(/var\(--text-3\)/)
    expect(root.getPropertyValue('--scrollbar-thumb-hover')).not.toBe(root.getPropertyValue('--scrollbar-thumb'))
  })

  it('WebView2 (Chromium) and WebKitGTK: thin, transparent track, rounded thumb, the same size on hover', () => {
    const bar = rule('::-webkit-scrollbar')
    expect(bar.getPropertyValue('width')).toBe('10px')
    expect(bar.getPropertyValue('height')).toBe('10px') // horizontal ones (code blocks, the full view strip) match
    expect(rule('::-webkit-scrollbar-track').getPropertyValue('background')).toBe('transparent')
    expect(rule('::-webkit-scrollbar-corner').getPropertyValue('background')).toBe('transparent')
    const thumb = rule('::-webkit-scrollbar-thumb')
    expect(thumb.getPropertyValue('background-color')).toBe('var(--scrollbar-thumb)')
    expect(thumb.getPropertyValue('border-radius')).not.toBe('')
    // a transparent border insets the thumb in its track
    expect(thumb.getPropertyValue('background-clip')).toBe('padding-box')
    const hover = rule('::-webkit-scrollbar-thumb:hover')
    expect(hover.getPropertyValue('background-color')).toBe('var(--scrollbar-thumb-hover)')
    // hover only recolours: no layout shift
    for (const p of ['width', 'height', 'border', 'border-width']) expect(hover.getPropertyValue(p)).toBe('')
    // no arrow buttons
    expect(rule('::-webkit-scrollbar-button').getPropertyValue('display')).toBe('none')
  })

  it('the standard properties only where the -webkit- ones are not supported (Chromium ignores those once these are set)', () => {
    const supports = [...sheet.cssRules].find((r): r is CSSSupportsRule => r instanceof CSSSupportsRule)
    expect(supports?.conditionText.replace(/\s+/g, ' ').trim()).toBe('not selector(::-webkit-scrollbar)')
    const all = styleRules(supports!.cssRules).find((r) => r.selectorText === '*')!.style
    expect(all.getPropertyValue('scrollbar-width')).toBe('thin')
    expect(all.getPropertyValue('scrollbar-color')).toBe('var(--scrollbar-thumb) transparent')
    // and not outside it
    expect(styleRules(sheet.cssRules).some((r) => r.style.getPropertyValue('scrollbar-width') || r.style.getPropertyValue('scrollbar-color'))).toBe(false)
  })

  it('no component styles its own scrollbar', () => {
    const files: string[] = []
    const walk = (dir: string) => {
      for (const e of readdirSync(dir, { withFileTypes: true })) {
        if (e.isDirectory()) walk(join(dir, e.name))
        else if (/\.(svelte|css)$/.test(e.name) && e.name !== 'app.css') files.push(join(dir, e.name))
      }
    }
    walk(__dirname)
    expect(files.length).toBeGreaterThan(10)
    expect(files.filter((f) => /scrollbar/.test(readFileSync(f, 'utf8'))).map((f) => f.slice(__dirname.length + 1))).toEqual([])
  })
})
