// Speaking rings (#651 §8.1): the ring's own compiled CSS keeps to the performance rules (only transform and opacity
// move, no filter, levels only through `scale`, animation only while on), and every style stays inside its 1.32 D
// ring box at its peak (Greg's rule: nothing spills).
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { compile } from 'svelte/compiler'
import { beforeAll, describe, expect, it } from 'vitest'

const scope = 'svelte-test'
let css = ''
beforeAll(() => {
  const src = readFileSync(resolve(__dirname, 'SpeakingRing.svelte'), 'utf8')
  css = compile(src, { filename: 'SpeakingRing.svelte', css: 'external', cssHash: () => scope }).css!.code
})

/** Top-level rules (selector, body) outside @keyframes, with @media bodies flattened in. */
function rules(): { sel: string; body: string }[] {
  const flat = css.replace(/@keyframes[^{]*\{(?:[^{}]*\{[^}]*\})*\s*\}/g, '').replace(/@media[^{]*\{((?:[^{}]*\{[^}]*\})*)\s*\}/g, '$1')
  return [...flat.matchAll(/([^{}]+)\{([^}]*)\}/g)].map((m) => ({ sel: m[1].replace(/\/\*[\s\S]*?\*\//g, '').trim(), body: m[2] }))
}
const decls = (body: string) =>
  body
    .split(';')
    .map((d) => d.trim())
    .filter(Boolean)
    .map((d) => [d.slice(0, d.indexOf(':')).trim(), d.slice(d.indexOf(':') + 1).trim()] as const)
function keyframes(): Map<string, string> {
  const out = new Map<string, string>()
  for (const m of css.matchAll(/@keyframes\s+([\w-]+)\s*\{((?:[^{}]*\{[^}]*\})*)\s*\}/g)) out.set(m[1].replace(/^svelte-test-/, ''), m[2])
  return out
}
/** The body of the first rule whose selector list contains exactly `sel` (scoped classes stripped). */
function ruleFor(sel: string): string {
  const strip = (s: string) => s.replaceAll(`:where(.${scope})`, '').replaceAll(`.${scope}`, '').replace(/\s+/g, ' ').trim()
  const r = rules().find((r) => r.sel.split(',').some((s) => strip(s) === sel))
  if (!r) throw new Error(`no rule for ${sel}`)
  return r.body
}
const num = (s: string) => Number(s)

describe('the ring CSS keeps to the performance rules (#651 §8.1)', () => {
  it('every_keyframes_body_sets_only_transform_and_opacity', () => {
    const kf = keyframes()
    expect([...kf.keys()].sort()).toEqual(['ring-b1', 'ring-b2', 'ring-b3', 'ring-bar', 'ring-dot', 'ring-halo', 'ring-pulse', 'ring-ripple', 'ring-snap', 'ring-spin'])
    for (const [name, body] of kf) {
      for (const step of body.matchAll(/\{([^}]*)\}/g)) for (const [p] of decls(step[1])) expect(['transform', 'opacity'], `${name}: ${p}`).toContain(p)
    }
  })

  it('every_transition_names_only_opacity_or_scale', () => {
    for (const { sel, body } of rules()) {
      for (const [p, v] of decls(body)) {
        if (p === 'transition-property') for (const n of v.split(',')) expect(['opacity', 'scale'], sel).toContain(n.trim())
        if (p !== 'transition') continue
        if (v === 'none') continue
        for (const part of v.split(',')) expect(['opacity', 'scale'], `${sel}: ${v}`).toContain(part.trim().split(/\s+/)[0])
      }
    }
  })

  it('no_filter_box_shadow_animation_or_will_change', () => {
    expect(css).not.toMatch(/(^|[^-])filter\s*:/)
    expect(css).not.toMatch(/backdrop-filter/)
    expect(css).not.toMatch(/will-change/)
    for (const { sel, body } of rules()) {
      if (!/box-shadow/.test(body)) continue
      expect(sel, 'box-shadow only on Classic').toMatch(/\.classic/)
      expect(body, sel).not.toMatch(/transition/)
    }
  })

  it('lvl_is_read_only_inside_scale', () => {
    for (const { sel, body } of rules()) {
      for (const [p, v] of decls(body)) {
        expect(p, sel).not.toBe('--lvl')
        if (v.includes('--lvl')) expect(p, `${sel}: ${p}`).toBe('scale')
        if (p === 'transition' || p === 'animation') expect(v).not.toMatch(/--lvl/)
      }
    }
    expect(css).not.toMatch(/@property/)
  })

  it('animations_run_only_when_on', () => {
    for (const { sel, body } of rules()) {
      for (const [p, v] of decls(body)) {
        if ((p === 'animation' || p === 'animation-name') && v !== 'none') {
          for (const s of sel.split(',')) expect(s, `${p}: ${v}`).toMatch(/\[data-ring=["']?on["']?\]/)
        }
      }
    }
  })
})

/** The largest uniform `scale(k)` in a keyframes body (1 when none). */
function peakScale(name: string): number {
  const body = keyframes().get(name)!
  const all = [...body.matchAll(/scale\(([\d.]+)\)/g)].map((m) => num(m[1]))
  return Math.max(1, ...all)
}
/** A layer's negative inset multiplier: `inset: calc(var(--d) * -k)`. */
function outset(sel: string): number {
  const m = ruleFor(sel).match(/inset:\s*calc\(var\(--d\)\s*\*\s*-([\d.]+)\)/)
  if (!m) throw new Error(`no negative inset on ${sel}`)
  return num(m[1])
}
/** A `scale: calc(a + b * var(--lvl, .5))` at --lvl 1, or 1 with none. */
function lvlPeak(sel: string): number {
  const m = ruleFor(sel).match(/scale:\s*(?:1\s+)?calc\(([\d.]+)\s*\+\s*([\d.]+)\s*\*\s*var\(--lvl/)
  return m ? num(m[1]) + num(m[2]) : 1
}
/** A `translate(x, y)` in a keyframes body, as D multiples (the largest offset). */
function peakTranslate(name: string): number {
  const body = keyframes().get(name)!
  const parts = [...body.matchAll(/calc\(var\(--d\)\s*\*\s*(-?[\d.]+)\)/g)].map((m) => Math.abs(num(m[1])))
  return Math.max(0, ...parts)
}

describe('no spill: every style fits its 1.32 D ring box (#651 §3.3)', () => {
  const HALF = 0.66 + 1e-3
  const table: { style: string; layer: string; kf: string | null }[] = [
    { style: 'glow', layer: '.glow', kf: null },
    { style: 'pulse', layer: '.pulse', kf: 'ring-pulse' },
    { style: 'ripple', layer: '.ripple', kf: 'ring-ripple' },
    { style: 'sweep', layer: '.sweep', kf: null },
    { style: 'orbit', layer: '.orbit', kf: null },
    { style: 'halo', layer: '.halo', kf: 'ring-halo' },
    { style: 'viewfinder', layer: '.vf', kf: 'ring-snap' },
  ]
  it.each(table)('every_style_stays_inside_the_ring_box: $style', ({ layer, kf }) => {
    const k = outset(layer)
    const peak = kf ? peakScale(kf) : 1
    expect((0.5 + k) * peak * lvlPeak(layer)).toBeLessThanOrEqual(HALF)
  })

  it('every_style_stays_inside_the_ring_box: liquid (its blobs drift as well as swell)', () => {
    const k = outset('.liquid')
    const wrapper = lvlPeak('.liquid')
    for (const b of ['ring-b1', 'ring-b2', 'ring-b3']) expect(((0.5 + k) * peakScale(b) + peakTranslate(b)) * wrapper, b).toBeLessThanOrEqual(HALF)
  })

  it('every_style_stays_inside_the_ring_box: wave of dots', () => {
    expect(outset('.dots')).toBe(0.16)
    const dot = ruleFor('.dots .dot')
    const top = num(dot.match(/top:\s*calc\(var\(--d\)\s*\*\s*([\d.]+)\)/)![1])
    const w = num(dot.match(/width:\s*max\([\d.]+px,\s*calc\(var\(--d\)\s*\*\s*([\d.]+)\)\)/)![1])
    const centre = 0.66 - top - w / 2
    expect(centre + (w / 2) * peakScale('ring-dot') * lvlPeak('.dots .dot')).toBeLessThanOrEqual(HALF)
  })

  it('every_style_stays_inside_the_ring_box: bars (their outer end is the box edge, they grow inward)', () => {
    expect(outset('.bars')).toBe(0.16)
    const bar = ruleFor('.bars .bar')
    expect(bar).toMatch(/top:\s*0/)
    expect(bar).toMatch(/transform-origin:\s*50% 100%/)
    const kfY = Math.max(...[...keyframes().get('ring-bar')!.matchAll(/scaleY\(([\d.]+)\)/g)].map((m) => num(m[1])))
    expect(kfY * lvlPeak('.bars .bar')).toBeLessThanOrEqual(1)
  })
})
