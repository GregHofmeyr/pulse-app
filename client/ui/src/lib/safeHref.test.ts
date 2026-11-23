import { describe, expect, it } from 'vitest'
import { safeHref } from './safeHref'

describe('safeHref', () => {
  it('keeps web links as given', () => {
    for (const ok of ['https://open.spotify.com/track/abc', 'http://example.com/', 'https://example.com:8443/a?b#c']) {
      expect(safeHref(ok)).toBe(ok)
    }
  })

  it('drops every other scheme and anything unparseable', () => {
    for (const bad of [
      'javascript:alert(1)',
      ' javascript:alert(1)',
      'JavaScript:alert(1)',
      'file:///C:/Windows/System32/calc.exe',
      '\\\\attacker.example\\share\\x.exe',
      'ms-settings:display',
      'data:text/html,hi',
      'vbscript:x',
      'steam://run/440',
      '/relative/path',
      'not a url',
      '',
      null,
      undefined,
    ]) {
      expect(safeHref(bad)).toBeUndefined()
    }
  })
})
