import { describe, expect, it } from 'vitest'
import { assetsInlineLimit } from '../vite.config'

// The app CSP is `font-src 'self'`, which blocks data: URIs, so fonts must stay real files.
describe('vite build config', () => {
  it('never inlines fonts, keeps the default for everything else', () => {
    expect(assetsInlineLimit('/x/fredoka-latin-ext-500-normal.woff2')).toBe(false)
    expect(assetsInlineLimit('/x/a.woff')).toBe(false)
    expect(assetsInlineLimit('/x/a.png')).toBeUndefined()
  })
})
