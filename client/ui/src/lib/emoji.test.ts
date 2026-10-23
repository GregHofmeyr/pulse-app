import { describe, expect, it } from 'vitest'
import { mediaUrl } from './emoji'

const hash = 'ab'.repeat(32)

describe('mediaUrl', () => {
  it('uses the http form of the scheme on Windows (WebView2)', () => {
    const ua = 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Edg/130.0'
    expect(mediaUrl(hash, ua)).toBe(`http://pulse-media.localhost/${hash}`)
  })

  it('uses the custom scheme elsewhere', () => {
    const ua = 'Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/605.1.15'
    expect(mediaUrl(hash, ua)).toBe(`pulse-media://localhost/${hash}`)
  })
})
