// Guards for the UI consistency pass (#173): rules that keep motion smooth and the look the same everywhere, read
// straight from app.css and every component's <style>, so drift can't come back unnoticed.
import { readdirSync, readFileSync } from 'node:fs'
import { join, resolve } from 'node:path'
import { describe, expect, it } from 'vitest'

const files = (() => {
  const out: string[] = []
  const walk = (dir: string) => {
    for (const e of readdirSync(dir, { withFileTypes: true })) {
      if (e.isDirectory()) walk(join(dir, e.name))
      else if (e.name.endsWith('.svelte')) out.push(join(dir, e.name))
    }
  }
  walk(__dirname)
  return out
})()
const rel = (f: string) => f.slice(__dirname.length + 1)
const noComments = (css: string) => css.replace(/\/\*[\s\S]*?\*\//g, '')
/** A component's CSS (its <style> blocks), or app.css whole. */
const cssOf = (f: string) =>
  noComments(f.endsWith('.css') ? readFileSync(f, 'utf8') : [...readFileSync(f, 'utf8').matchAll(/<style[^>]*>([\s\S]*?)<\/style>/g)].map((m) => m[1]).join('\n'))
const ALL = [...files, resolve(__dirname, 'app.css')]
/** Every `prop: value` declaration in the CSS whose property matches. */
const decls = (css: string, prop: RegExp) => [...css.matchAll(/(?:^|[\s;{])([\w-]+)\s*:\s*([^;{}]+)/g)].filter((m) => prop.test(m[1])).map((m) => ({ prop: m[1], value: m[2].trim() }))

describe('rendering safety', () => {
  // app.css bans it: an older WebKitGTK drops the whole declaration (a border, a fill or a highlight vanished)
  it('no color-mix() anywhere: translucent colours are theme tokens', () => {
    const offenders = ALL.filter((f) => /color-mix\(/.test(cssOf(f)) || (f.endsWith('.svelte') && /color-mix\(/.test(noComments(readFileSync(f, 'utf8'))))).map(rel)
    expect(offenders).toEqual([])
  })
})

describe('motion', () => {
  // Layout properties re-run layout on every frame, and box-shadow repaints: they stutter on a busy machine. Meters and
  // fills scale (scaleX), knobs slide (translateX), rings fade (opacity).
  const LAYOUT = /^(width|height|min-width|max-width|min-height|max-height|left|top|right|bottom|margin|padding|box-shadow|all)(-|$)/
  it('nothing transitions width, height, left, top, margins, padding, box-shadow or `all`', () => {
    const offenders: string[] = []
    for (const f of ALL) {
      for (const d of decls(cssOf(f), /^transition(-property)?$/)) {
        for (const part of d.value.split(',')) {
          const name = part.trim().split(/\s+/)[0]
          if (LAYOUT.test(name)) offenders.push(`${rel(f)}: ${d.prop}: ${d.value}`)
        }
      }
    }
    expect(offenders).toEqual([])
  })

  // The celebrations keep their own stepped timings (they are flourishes, not UI motion), and a spinner its own speed.
  const OWN_TIMING: Record<string, RegExp> = {
    'components/QotyCard.svelte': /^qoty-/,
    'components/QotyVoting.svelte': /^(qoty-|transform .35s steps)/,
    'components/BadgeToast.svelte': /^confetti-burst/,
    // the joining spinners, and their slower turn under reduced motion
    'components/ChannelList.svelte': /^(spin |2\.4s$)/,
    'components/UserTile.svelte': /^(spin |2\.4s$)/,
    // the jumped-to message's highlight fades over two seconds, long enough to find it
    'components/TextChannel.svelte': /^jump-flash /,
    // reduced motion: everything finishes at once
    'app.css': /^\.01ms !important$/,
  }
  it('every transition and animation takes a motion token (--dur-fast, --dur, --dur-slow, --dur-exit), not a raw duration', () => {
    const offenders: string[] = []
    for (const f of ALL) {
      for (const d of decls(cssOf(f), /^(transition|animation)(-duration)?$/)) {
        if (d.value === 'none') continue
        if (!/(^|[\s,(])\d*\.?\d+m?s\b/.test(d.value)) continue
        if (OWN_TIMING[rel(f)]?.test(d.value)) continue
        offenders.push(`${rel(f)}: ${d.prop}: ${d.value}`)
      }
    }
    expect(offenders).toEqual([])
  })

  it('app.css defines the motion tokens, exits quicker than entrances', () => {
    const css = readFileSync(resolve(__dirname, 'app.css'), 'utf8')
    const ms = (name: string) => Number(new RegExp(`${name}: (\\d+)ms;`).exec(css)?.[1])
    expect(ms('--dur-fast')).toBeLessThan(ms('--dur'))
    expect(ms('--dur')).toBeLessThan(ms('--dur-slow'))
    expect(ms('--dur-exit')).toBeLessThan(ms('--dur'))
    expect(css).toMatch(/--ease: cubic-bezier\(/)
    expect(css).toMatch(/--ease-exit: cubic-bezier\(/)
  })

  it('buttons, menu items, options and hover rows share one hover fade, filter included (the accent buttons brighten)', () => {
    const css = noComments(readFileSync(resolve(__dirname, 'app.css'), 'utf8'))
    const rule = /(^|\n)button, \.menu-item, \[role='option'\], \.hover-row \{ transition: ([^}]*)\}/.exec(css)
    expect(rule).not.toBeNull()
    for (const p of ['background-color', 'color', 'border-color', 'opacity', 'filter']) expect(rule![2]).toContain(`${p} var(--dur-fast) var(--ease)`)
  })

  it('menus and popovers grow from their --pop-origin', () => {
    const css = noComments(readFileSync(resolve(__dirname, 'app.css'), 'utf8'))
    expect(css).toMatch(/\.pop-in \{[^}]*transform-origin: var\(--pop-origin\)/)
  })
})

describe('one tooltip path', () => {
  // The themed tooltip (lib/tooltip.ts) shows aria-labels and data-tip; a native title= would add the OS tooltip a
  // second later, sometimes with other words (#173). Only an iframe keeps its title (its accessible name).
  it('no element has a title= attribute (data-tip instead)', () => {
    const offenders: string[] = []
    for (const f of files) {
      const markup = readFileSync(f, 'utf8').replace(/<script[\s\S]*?<\/script>/g, '').replace(/<style[\s\S]*?<\/style>/g, '')
      for (const m of markup.matchAll(/<([a-z][\w-]*)\b[^>]*?\stitle=/g)) {
        // the People board's disabled "Call" placeholder, which the DM calls work replaces
        if (m[1] === 'iframe' || (rel(f) === 'components/PeopleBoard.svelte' && /Calls arrive soon/.test(m[0] + markup.slice(m.index!, m.index! + 200)))) continue
        offenders.push(`${rel(f)}: <${m[1]} … title=`)
      }
    }
    expect(offenders).toEqual([])
  })
})

describe('radii', () => {
  // app.css's shared classes use the radius tokens; components still being moved over are listed (#173 follow-up).
  it('app.css writes no raw 6/8/10/12/16px radius (they are --radius-xs … --radius-xl)', () => {
    const offenders = decls(cssOf(resolve(__dirname, 'app.css')), /^border-radius$/).filter((d) => /\b(6|8|10|12|16)px\b/.test(d.value))
    expect(offenders).toEqual([])
  })
})
