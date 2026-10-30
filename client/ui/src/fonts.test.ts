/// <reference types="node" />
// Bundled fonts: latin scripts only, woff2 only, and for those glyphs the same faces as before.
import { readFileSync } from 'node:fs'
import { createRequire } from 'node:module'
import { describe, expect, it } from 'vitest'

const read = (rel: string) => readFileSync(new URL(rel, import.meta.url), 'utf8')
const require = createRequire(import.meta.url)
const upstreamCss = (weight: string) => readFileSync(require.resolve(`@fontsource/onest/${weight}.css`), 'utf8')
const WEIGHTS = ['400', '500', '600', '700']
const SUBSETS = ['latin', 'latin-ext', 'symbols']

const faces = (css: string) =>
  [...css.matchAll(/@font-face\s*{([^}]*)}/g)].map(([, body]) => ({
    weight: /font-weight:\s*(\d+)/.exec(body)?.[1],
    urls: [...body.matchAll(/url\(['"]?([^'")]+)/g)].map((m) => m[1]),
    range: /unicode-range:\s*([^;]+);/.exec(body)?.[1].trim(),
  }))
const subsetOf = (url: string) => /onest-(.+)-\d{3}-normal/.exec(url)?.[1]

describe('bundled fonts', () => {
  it('main.ts loads fonts.css instead of the full @fontsource sets', () => {
    const main = read('./main.ts')
    expect(main).toContain("import './fonts.css'")
    expect(main).not.toMatch(/@fontsource\/onest\/\d+\.css/)
  })

  it('ships only latin, latin-ext and symbols, as woff2, each once for each of the four weights the UI uses', () => {
    const ours = faces(read('./fonts.css'))
    for (const f of ours) {
      expect(f.urls).toHaveLength(1)
      expect(f.urls[0]).toMatch(/\.woff2$/)
      // the face's file is the one for its own weight
      expect(f.urls[0]).toContain(`-${f.weight}-normal.`)
    }
    const pairs = ours.map((f) => `${f.weight} ${subsetOf(f.urls[0])}`).sort()
    expect(pairs).toEqual(WEIGHTS.flatMap((w) => SUBSETS.map((sub) => `${w} ${sub}`)).sort())
  })

  it('every file it points at is in the package', () => {
    for (const f of faces(read('./fonts.css'))) expect(() => require.resolve(f.urls[0])).not.toThrow()
  })

  it('uses the same unicode ranges as @fontsource, weight by weight, so those glyphs come from the same face', () => {
    const ranges = new Map(WEIGHTS.flatMap((w) => faces(upstreamCss(w)).map((f) => [`${w} ${subsetOf(f.urls[0])}`, f.range])))
    for (const f of faces(read('./fonts.css'))) {
      const want = ranges.get(`${f.weight} ${subsetOf(f.urls[0])}`)
      expect(want).toBeTruthy()
      expect(f.range).toBe(want)
    }
  })
})
