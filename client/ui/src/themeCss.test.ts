// @vitest-environment jsdom
// The theme blocks in app.css (#42 step 2): each theme declares every colour token (so a picker preview with its own
// data-theme shows that theme, whatever <html> is on), lib/themes.ts repeats their surfaces faithfully, and every text
// token reads on every surface it sits on.
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { beforeAll, describe, expect, it } from 'vitest'
import { contrast } from './lib/color'
import { deriveAccent, surfacesFor, THEMES, TINTS, type ThemeKey, type TintKey } from './lib/themes'

let rules: CSSStyleRule[]
beforeAll(() => {
  const style = document.createElement('style')
  style.textContent = readFileSync(resolve(__dirname, 'app.css'), 'utf8')
  document.head.append(style)
  rules = [...style.sheet!.cssRules].filter((r): r is CSSStyleRule => r instanceof CSSStyleRule)
})

const attr = (sel: string, name: string) => new RegExp(`\\[data-${name}=["']([a-z]+)["']\\]`).exec(sel)?.[1] ?? null
/** The rule whose selector is exactly this theme (and tint). */
function block(theme: ThemeKey, tint?: TintKey): CSSStyleDeclaration {
  const found = rules.filter((r) => r.selectorText.split(',').some((s) => attr(s, 'theme') === theme && attr(s, 'tint') === (tint ?? null)))
  expect(found.length, `one block for ${theme}${tint ? ' / ' + tint : ''}`).toBe(1)
  return found[0].style
}
/** jsdom drops the spaces after commas, and `#fff` is `#ffffff`: the form lib/themes.ts writes. */
const hex6 = (v: string) => v.replace(/,(?=\S)/g, ', ').replace(/^#([0-9a-f])([0-9a-f])([0-9a-f])$/i, '#$1$1$2$2$3$3')
const names = (d: CSSStyleDeclaration) => [...d].filter((p) => p.startsWith('--')).sort()

/** A theme's tokens (Light's with a tint on top), `var()` references resolved. */
function tokens(theme: ThemeKey, tint: TintKey = 'plain'): (name: string) => string {
  const layers = [block(theme)]
  if (theme === 'light' && tint !== 'plain') layers.push(block('light', tint))
  const raw = (n: string): string => {
    for (const l of [...layers].reverse()) {
      const v = l.getPropertyValue(n).trim()
      if (v) return v
    }
    throw new Error(`${theme}/${tint}: no ${n}`)
  }
  const get = (n: string): string => {
    const v = raw(n)
    const ref = /^var\((--[\w-]+)\)$/.exec(v)
    return ref ? get(ref[1]) : hex6(v)
  }
  return get
}

const VARIANTS: [ThemeKey, TintKey][] = THEMES.flatMap((t) => (t.key === 'light' ? TINTS.map((n) => [t.key, n.key] as [ThemeKey, TintKey]) : [[t.key, 'plain'] as [ThemeKey, TintKey]]))
const TINT_TOKENS = ['--bg-0', '--bg-1', '--bg-3', '--bg-4', '--bg-inset', '--line', '--line-soft', '--line-menu', '--line-strong', '--scrollbar-thumb', '--scrollbar-thumb-hover'].sort()

describe('theme blocks', () => {
  it('Pulse Dark is also the plain :root (the default with no data-theme)', () => {
    const dark = rules.filter((r) => r.selectorText.split(',').some((s) => attr(s, 'theme') === 'dark'))
    expect(dark[0].selectorText.split(',').map((s) => s.trim())).toContain(':root')
  })

  it('every theme declares the same colour tokens as Pulse Dark', () => {
    const want = names(block('dark'))
    expect(want.length).toBeGreaterThan(50)
    for (const t of THEMES) expect(names(block(t.key)), t.key).toEqual(want)
  })

  it("Light's tints change surfaces and lines only (never text)", () => {
    for (const n of TINTS.filter((n) => n.key !== 'plain')) expect(names(block('light', n.key)), n.key).toEqual(TINT_TOKENS)
  })

  it('color-scheme follows light and dark', () => {
    for (const t of THEMES) expect(block(t.key).getPropertyValue('color-scheme'), t.key).toBe(t.light ? 'light' : 'dark')
  })

  it.each(VARIANTS)('lib/themes.ts has the same surfaces as app.css (%s, %s)', (theme, tint) => {
    const tk = tokens(theme, tint)
    const s = surfacesFor(theme, tint)
    expect(s).toEqual({ bg0: tk('--bg-0'), bg1: tk('--bg-1'), bg2: tk('--bg-2'), bg3: tk('--bg-3'), menu: tk('--surface-menu') })
  })

  it.each(THEMES.map((t) => t.key))("%s: the accent family in CSS is deriveAccent of the theme's own accent", (theme) => {
    const t = THEMES.find((x) => x.key === theme)!
    const tk = tokens(theme)
    expect(tk('--accent')).toBe(t.accent)
    for (const [name, value] of Object.entries(deriveAccent(t.accent, surfacesFor(theme, 'plain'), t.light))) expect(tk(name), `${theme} ${name}`).toBe(value)
  })

  it('the Quote of the Year island and the tooltip are the same in every theme (fixed dark boxes)', () => {
    const fixed = ['--quote-surface', '--island-text', '--island-text-2', '--island-text-3', '--tooltip-bg', '--tooltip-text', '--scrim-strong']
    const dark = tokens('dark')
    for (const t of THEMES) for (const n of fixed) expect(tokens(t.key)(n), `${t.key} ${n}`).toBe(dark(n))
  })
})

describe('contrast, every theme and tint', () => {
  // Primary text reads at 4.5:1 everywhere, including a hovered row (--bg-4).
  const PRIMARY = ['--text', '--text-body', '--text-2']
  // Secondary text, semantic text and accent text: 4.5:1 on the window, sidebar, chat and inset; on raised and hovered
  // rows and menus at least 3:1 (they're short labels and meta there). New themes hold 4.5:1 on raised rows and menus
  // too; Pulse Dark keeps today's approved values (text-3 4.49, danger-text 4.45 on --bg-3; 4.20–4.25 on menus).
  const SECONDARY = ['--text-row', '--text-3', '--danger-text', '--warn-text', '--ok-text', '--quote-green-text', '--accent-text']
  const BASE = ['--bg-0', '--bg-1', '--bg-2', '--bg-inset']
  const RAISED = ['--bg-3', '--surface-menu']

  it.each(VARIANTS)('%s (%s)', (theme, tint) => {
    const tk = tokens(theme, tint)
    const fails: string[] = []
    const need = (fg: string, bg: string, ratio: number) => {
      const c = contrast(tk(fg), tk(bg))
      if (c < ratio) fails.push(`${fg} on ${bg}: ${c.toFixed(2)} < ${ratio}`)
    }
    for (const fg of PRIMARY) for (const bg of [...BASE, ...RAISED, '--bg-4']) need(fg, bg, 4.5)
    for (const fg of SECONDARY) {
      for (const bg of BASE) need(fg, bg, 4.5)
      for (const bg of RAISED) need(fg, bg, theme === 'dark' ? 3 : 4.5)
      need(fg, '--bg-4', 3)
    }
    need('--on-accent', '--accent', 4.5)
    need('--on-danger', '--danger', 4.5)
    need('--on-quote-green', '--quote-green', 4.5)
    need('--tooltip-text', '--tooltip-bg', 4.5)
    expect(fails).toEqual([])
  })
})
