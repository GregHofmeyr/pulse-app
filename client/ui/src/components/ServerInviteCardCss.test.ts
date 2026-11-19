// The opened invite card stays crisp (#517): it's laid out at the size it's seen at, so at rest it has no transform
// (an upscaled `scale()` blurred its text and icon), nothing keeps a compositing layer on after the rise, and the
// icon is drawn from the stored 256px picture (2× and more of the 94px it shows at).
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { describe, expect, it } from 'vitest'

const file = readFileSync(resolve(__dirname, 'ServerInviteCard.svelte'), 'utf8')
const css = /<style>([\s\S]*)<\/style>/.exec(file)![1].replace(/\/\*[\s\S]*?\*\//g, '')
/** The declarations of the rule whose selector is exactly `selector`. */
const rule = (selector: string) => {
  const m = new RegExp(`(?:^|\\n)\\s*${selector.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}\\s*{([^}]*)}`).exec(css)
  expect(m, selector).not.toBeNull()
  return m![1]
}

describe('the opened invite card is crisp (#517)', () => {
  it('rests untransformed; only the pack and rip states scale it, and down, never up', () => {
    expect(rule('.card')).toMatch(/transform:\s*none;/)
    for (const s of [...css.matchAll(/scale\(([\d.]+)\)/g)].filter((m) => !/@keyframes pack-burst/.test(css.slice(css.lastIndexOf('@keyframes', m.index), m.index))))
      expect(Number(s[1]), s[0]).toBeLessThanOrEqual(1)
  })

  it('declares no will-change, and the icon ring is centred without a transform', () => {
    expect(css).not.toMatch(/will-change/)
    expect(rule('.ring')).not.toMatch(/transform/)
  })

  it('MYTHICAL PULL takes the per-theme gold text token (4.5:1 checked in themeCss.test.ts)', () => {
    expect(rule('.rarity')).toMatch(/color:\s*var\(--pack-gold-text\);/)
  })
})
