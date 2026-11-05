import { describe, expect, it } from 'vitest'
import { NAME_FONTS, fontByKey } from './nameFonts'

// A literal copy of server/src/profiles/mod.rs NAME_FONTS (asserted there too): change one side
// and both tests fail.
const SERVER_KEYS = [
  'cormorant-garamond',
  'fredoka',
  'press-start-2p',
  'pacifico',
  'caveat',
  'space-grotesk',
  'bricolage-grotesque',
  'orbitron',
  'bungee',
  'silkscreen',
  'audiowide',
  'righteous',
  'kaushan-script',
  'black-ops-one',
]

describe('name fonts', () => {
  it('fourteen fonts with unique keys and labels', () => {
    expect(NAME_FONTS).toHaveLength(14)
    expect(new Set(NAME_FONTS.map((f) => f.key)).size).toBe(14)
    expect(new Set(NAME_FONTS.map((f) => f.label)).size).toBe(14)
    expect(NAME_FONTS.some((f) => f.label === 'Default')).toBe(false)
  })

  it('every size factor is a sane number (names line up, none tiny or huge)', () => {
    for (const f of NAME_FONTS) {
      expect(f.scale, f.key).toBeGreaterThanOrEqual(0.6)
      expect(f.scale, f.key).toBeLessThanOrEqual(1.3)
    }
  })

  it('keys match the server list', () => {
    expect(NAME_FONTS.map((f) => f.key)).toEqual(SERVER_KEYS)
  })

  it('rows follow the plan table', () => {
    expect(NAME_FONTS.map((f) => [f.label, f.weight, f.scale])).toEqual([
      ['Elegant', 600, 1.15],
      ['Rounded', 500, 1.0],
      ['Pixel', 400, 0.68],
      ['Script', 400, 0.95],
      ['Handwritten', 600, 1.2],
      ['Grotesque', 600, 1.0],
      ['Quirky', 600, 1.0],
      ['Techy', 600, 0.86],
      // #142: starting points from each face's cap and x-height (and width) against Onest
      ['Bold', 400, 0.82],
      ['8-bit', 400, 0.85],
      ['Racing', 400, 0.92],
      ['Retro', 400, 1.0],
      ['Brush', 400, 1.1],
      ['Stencil', 400, 0.92],
    ])
    expect(fontByKey('black-ops-one')?.family).toBe("'Black Ops One', 'Onest', system-ui, sans-serif")
    expect(fontByKey('pacifico')?.family).toBe("'Pacifico', 'Onest', system-ui, sans-serif")
  })

  it('fontByKey(null) is null', () => {
    expect(fontByKey(null)).toBeNull()
    expect(fontByKey('nope')).toBeNull()
  })
})
