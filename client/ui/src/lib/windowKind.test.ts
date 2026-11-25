import { describe, expect, it } from 'vitest'
import { windowKind } from './windowKind'

const at = (search: string) => ({ search }) as Location

describe('windowKind', () => {
  it('windowKind reads ?window=build', () => {
    expect(windowKind(at('?window=build'))).toBe('build')
  })

  it('anything else is the main window', () => {
    for (const s of ['', '?', '?window=main', '?window=Build', '?window=build2', '?other=build', '?window=']) {
      expect(windowKind(at(s)), s).toBe('main')
    }
  })
})
