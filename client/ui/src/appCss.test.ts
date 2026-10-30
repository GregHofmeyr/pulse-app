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
    // --text-3 (#8e92a0) at two opacities, as plain rgb(): no color-mix() for an engine to drop
    expect(root.getPropertyValue('--text-3')).toBe('#8e92a0')
    const alpha = (v: string) => Number(/^rgb\(142 146 160 ?\/ ?(\.\d+)\)$/.exec(v.trim())?.[1])
    const [rest, hover] = [alpha(root.getPropertyValue('--scrollbar-thumb')), alpha(root.getPropertyValue('--scrollbar-thumb-hover'))]
    expect(rest).toBeGreaterThan(0)
    expect(hover).toBeGreaterThan(rest)
    expect(hover).toBeLessThan(1)
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
    expect(files.filter((f) => /::-webkit-scrollbar|scrollbar-(width|color)/.test(readFileSync(f, 'utf8'))).map((f) => f.slice(__dirname.length + 1))).toEqual([])
  })
})

describe('motion', () => {
  it('buttons ease colours and opacity only, never a property that moves layout', () => {
    const button = styleRules(sheet.cssRules).filter((r) => r.selectorText === 'button').map((r) => r.style.getPropertyValue('transition')).find(Boolean)
    expect(button).toBeDefined()
    const props = button!.split(',').map((t) => t.trim().split(/\s+/)[0])
    expect(props.sort()).toEqual(['background-color', 'border-color', 'color', 'opacity'])
  })

  it('reduced motion finishes animations and transitions at once, but leaves the spinners to slow down', () => {
    const media = [...sheet.cssRules].find((r): r is CSSMediaRule => r instanceof CSSMediaRule && /prefers-reduced-motion: ?reduce/.test(r.conditionText))
    expect(media).toBeDefined()
    const all = styleRules(media!.cssRules)[0]
    expect(all.selectorText).toContain('*:not(.spinner)')
    expect(all.style.getPropertyValue('animation-duration')).toBe('0.01ms')
    expect(all.style.getPropertyPriority('animation-duration')).toBe('important')
    expect(all.style.getPropertyValue('transition-duration')).toBe('0.01ms')
  })

  it('while streaming or unfocused, transitions snap (nothing can be left mid-fade)', () => {
    const pause = styleRules(sheet.cssRules).find((r) => r.selectorText.includes('.streaming *'))!
    expect(pause.selectorText).toContain('.unfocused *')
    expect(pause.style.getPropertyValue('transition')).toBe('none')
    expect(pause.style.getPropertyPriority('transition')).toBe('important')
  })
})
