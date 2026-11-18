// Name effects (#468): the eight effects, their keys (the server's list), and the colours each draws from.
import { describe, expect, it } from 'vitest'
import { effectByKey, effectCss, letters, NAME_EFFECTS, popColors } from './nameEffects'
import type { Paint } from './nameStyle'

const solid: Paint = { kind: 'solid', color: '#a3d8ff' }
const gradient: Paint = { kind: 'gradient', from: '#ffbe7a', to: '#ff7fae' }
const plain: Paint = { kind: 'plain' }
const vars = (css: string[]) => Object.fromEntries(css.filter((d) => d.startsWith('--')).map((d) => d.split(': ')))
const fx = (key: string) => effectByKey(key)!

describe('name effects (#468)', () => {
  /** A literal copy of NAME_EFFECTS in server/src/profiles/mod.rs: change one side and both tests fail. */
  it('the eight effects, keyed as the server keys them', () => {
    expect(NAME_EFFECTS.map((e) => e.key)).toEqual(['neon', 'toon', 'pop', 'gummy', 'prism', 'shimmer', 'glitch', 'ember'])
    expect(NAME_EFFECTS.map((e) => e.label)).toEqual(['Neon', 'Toon', 'Pop', 'Gummy', 'Prism', 'Shimmer', 'Glitch', 'Ember'])
  })

  it('an unknown or missing key is no effect', () => {
    expect(effectByKey('sparkle')).toBeNull()
    expect(effectByKey(null)).toBeNull()
    expect(effectByKey(undefined)).toBeNull()
  })

  it('Neon, Toon, Pop, Gummy, Shimmer and Glitch draw from the name colour', () => {
    for (const key of ['neon', 'toon', 'pop', 'gummy', 'shimmer', 'glitch']) {
      const v = vars(effectCss(fx(key), solid).css)
      expect(v['--fx-a'], key).toBe('#a3d8ff')
    }
    // a gradient's second stop is the partner colour (Toon's shadow)
    expect(vars(effectCss(fx('toon'), gradient).css)['--fx-b']).toBe('#ff7fae')
    // Gummy and Shimmer fill the letters from it
    expect(effectCss(fx('gummy'), solid).css.join(';')).toContain('#a3d8ff 55%')
    expect(effectCss(fx('shimmer'), gradient).css.join(';')).toMatch(/#ffbe7a 0%.*#ff7fae 100%/)
  })

  it('Prism keeps its rainbow and Ember its fire, whatever the name colour', () => {
    for (const p of [solid, gradient, plain]) {
      expect(effectCss(fx('prism'), p).css.join(';')).toContain('#ff5f6d, #ffc371, #47e6a0, #4fa8ff, #b16cff, #ff5f6d')
      expect(effectCss(fx('prism'), p).replacesPaint).toBe(true)
      const ember = effectCss(fx('ember'), p)
      expect(ember.css.join(';')).toContain('#ff3d00')
      // the charcoal ash that sets it apart from an orange Neon
      expect(vars(ember.css)['--fx-ash']).toMatch(/^#[0-9a-f]{6}$/)
    }
  })

  it('a name in the plain text colour still gets every effect, from the colour it sits in', () => {
    for (const e of NAME_EFFECTS) {
      const r = effectCss(e, plain)
      expect(vars(r.css)['--fx-a'], e.key).toBeUndefined()
    }
    expect(effectCss(fx('shimmer'), plain).css.join(';')).toContain('currentColor 0%')
  })

  it("Pop's letters: graphemes kept whole, and a gradient spread across them", () => {
    expect(letters('jo👍🏽')).toEqual(['j', 'o', '👍🏽'])
    expect(popColors(gradient, 3)).toEqual(['#ffbe7a', '#ff9f94', '#ff7fae'])
    expect(popColors(solid, 2)).toEqual([null, null])
    expect(effectCss(fx('pop'), gradient).replacesPaint).toBe(true)
    expect(effectCss(fx('pop'), solid).replacesPaint).toBe(false)
  })
})
