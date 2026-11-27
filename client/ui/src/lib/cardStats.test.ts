import { readFileSync } from 'node:fs'
import { describe, expect, it } from 'vitest'
import { CARD_STATS } from './cardStats'

describe('card stats (#727)', () => {
  it('the keys and their order are the server\'s CARD_STATS', () => {
    const src = readFileSync(new URL('../../../../server/src/profiles/mod.rs', import.meta.url), 'utf8')
    const m = src.match(/pub const CARD_STATS: \[&str; \d+\] = \[([^\]]*)\]/)
    expect(m, 'CARD_STATS not found in server/src/profiles/mod.rs').not.toBeNull()
    expect([...m![1].matchAll(/"([^"]+)"/g)].map((x) => x[1])).toEqual([...CARD_STATS])
  })
})
