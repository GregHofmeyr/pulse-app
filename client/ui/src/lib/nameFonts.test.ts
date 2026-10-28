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
]

describe('name fonts', () => {
  it('eight fonts with unique keys', () => {
    expect(NAME_FONTS).toHaveLength(8)
    expect(new Set(NAME_FONTS.map((f) => f.key)).size).toBe(8)
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
    ])
    expect(fontByKey('pacifico')?.family).toBe("'Pacifico', 'Onest', system-ui, sans-serif")
  })

  it('fontByKey(null) is null', () => {
    expect(fontByKey(null)).toBeNull()
    expect(fontByKey('nope')).toBeNull()
  })
})
