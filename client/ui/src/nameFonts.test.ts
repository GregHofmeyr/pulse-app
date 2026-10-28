/// <reference types="node" />
// Bundled name fonts: one weight per family, latin (+ latin-ext) woff2 only, same ranges as the packages.
import { readFileSync } from 'node:fs'
import { createRequire } from 'node:module'
import { describe, expect, it } from 'vitest'

const read = (rel: string) => readFileSync(new URL(rel, import.meta.url), 'utf8')
const require_ = createRequire(import.meta.url)
const pkgFile = (pkg: string, file: string) =>
  readFileSync(require_.resolve(`@fontsource/${pkg}/${file}`), 'utf8')

// pkg, family, weight, subsets (the plan's reconciliation 9)
const FONTS = [
  ['cormorant-garamond', 'Cormorant Garamond', 600, ['latin', 'latin-ext']],
  ['fredoka', 'Fredoka', 500, ['latin', 'latin-ext']],
  ['press-start-2p', 'Press Start 2P', 400, ['latin', 'latin-ext']],
  ['pacifico', 'Pacifico', 400, ['latin', 'latin-ext']],
  ['caveat', 'Caveat', 600, ['latin', 'latin-ext']],
  ['space-grotesk', 'Space Grotesk', 600, ['latin', 'latin-ext']],
  ['bricolage-grotesque', 'Bricolage Grotesque', 600, ['latin', 'latin-ext']],
  ['orbitron', 'Orbitron', 600, ['latin']],
] as const

const faces = (css: string) =>
  [...css.matchAll(/@font-face\s*{([^}]*)}/g)].map(([, body]) => ({
    family: /font-family:\s*'([^']+)'/.exec(body)?.[1],
    weight: /font-weight:\s*(\d+)/.exec(body)?.[1],
    display: /font-display:\s*(\w+)/.exec(body)?.[1],
    urls: [...body.matchAll(/url\(['"]?([^'")]+)/g)].map((m) => m[1]),
    range: /unicode-range:\s*([^;]+);/.exec(body)?.[1].trim(),
  }))

describe('bundled name fonts', () => {
  const ours = faces(read('./nameFonts.css'))

  it('main.ts imports nameFonts.css', () => {
    expect(read('./main.ts')).toContain("import './nameFonts.css'")
  })

  it('one weight per family, woff2 only, latin and latin-ext only', () => {
    expect(ours).toHaveLength(15)
    for (const [pkg, family, weight, subsets] of FONTS) {
      const mine = ours.filter((f) => f.family === family)
      expect(mine, family).toHaveLength(subsets.length)
      const urls = mine.map((f) => f.urls[0]).sort()
      expect(urls).toEqual(
        subsets
          .map((s) => `@fontsource/${pkg}/files/${pkg}-${s}-${weight}-normal.woff2`)
          .sort(),
      )
      for (const f of mine) {
        expect(f.urls).toHaveLength(1)
        expect(f.weight).toBe(String(weight))
        expect(f.display).toBe('swap')
      }
    }
  })

  it("unicode ranges match each package's own css", () => {
    for (const [pkg, family, weight, subsets] of FONTS) {
      // The packages' per-subset files carry no unicode-range; the per-weight file does.
      const upstream = faces(pkgFile(pkg, `${weight}.css`))
      for (const s of subsets) {
        const sub = faces(pkgFile(pkg, `${s}-${weight}.css`))[0]
        expect(sub.urls[0], `${pkg} ${s}`).toContain(`${pkg}-${s}-${weight}-normal.woff2`)
        const want = upstream.find((f) => f.urls[0].includes(`-${s}-${weight}-normal`))
        expect(want?.range, `${pkg} ${s}`).toBeTruthy()
        const mine = ours.find((f) => f.family === family && f.urls[0].includes(`-${s}-${weight}-normal`))
        expect(mine?.range, `${pkg} ${s}`).toBe(want?.range)
      }
    }
  })

  it('every bundled family has a THIRD_PARTY entry with OFL-1.1 and 5.3.0', () => {
    const doc = readFileSync(new URL('../../../THIRD_PARTY.md', import.meta.url), 'utf8')
    for (const [pkg, family] of FONTS) {
      const entry = doc.split(/^### /m).find((s) => s.startsWith(family))
      expect(entry, family).toBeTruthy()
      expect(entry).toContain(`@fontsource/${pkg}`)
      expect(entry).toContain('5.3.0')
      expect(entry).toContain('SIL Open Font License 1.1')
      expect(entry).not.toMatch(/@[\w-]+\.\w+/) // no email address
    }
    const onest = doc.split(/^### /m).find((s) => s.startsWith('Onest'))
    expect(onest).toContain('@fontsource/onest')
    expect(onest).toContain('SIL Open Font License 1.1')
  })
})

describe('NAME_FONTS table vs nameFonts.css', () => {
  it('each family and weight appears as a font-family/font-weight pair in the css', async () => {
    const { NAME_FONTS } = await import('./lib/nameFonts')
    const pairs = new Set(
      faces(read('./nameFonts.css')).map((f) => `${f.family}|${f.weight}`),
    )
    for (const f of NAME_FONTS) {
      const family = /^'([^']+)'/.exec(f.family)?.[1]
      expect(pairs.has(`${family}|${f.weight}`), f.key).toBe(true)
    }
  })
})
