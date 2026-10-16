/// <reference types="node" />
// Bundled fonts: latin scripts only, woff2 only, and for those glyphs the same faces as before.
import { readFileSync } from 'node:fs'
import { createRequire } from 'node:module'
import { describe, expect, it } from 'vitest'

const read = (rel: string) => readFileSync(new URL(rel, import.meta.url), 'utf8')
const upstream = readFileSync(createRequire(import.meta.url).resolve('@fontsource/onest/400.css'), 'utf8')

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

  it('ships only latin, latin-ext and symbols, as woff2, for the four weights the UI uses', () => {
    const ours = faces(read('./fonts.css'))
    expect(ours).toHaveLength(12)
    for (const f of ours) {
      expect(f.urls).toHaveLength(1)
      expect(f.urls[0]).toMatch(/\.woff2$/)
      expect(['latin', 'latin-ext', 'symbols']).toContain(subsetOf(f.urls[0]))
    }
    expect(new Set(ours.map((f) => f.weight))).toEqual(new Set(['400', '500', '600', '700']))
  })

  it('uses the same unicode ranges as @fontsource, so those glyphs come from the same face', () => {
    const ranges = new Map(faces(upstream).map((f) => [subsetOf(f.urls[0]), f.range]))
    for (const f of faces(read('./fonts.css'))) expect(f.range).toBe(ranges.get(subsetOf(f.urls[0])))
  })
})
