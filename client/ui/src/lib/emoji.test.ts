import { readdirSync } from 'node:fs'
import { createRequire } from 'node:module'
import { dirname } from 'node:path'
import { describe, expect, it } from 'vitest'
import { byShortcode, canonicalKey, customToken, groups, mediaUrl, reactionKey, search, standard, TOKEN_RE, twemojiUrl, withSkin } from './emoji'
import type { CustomEmoji } from './protocol/CustomEmoji'

const custom = (name: string, over: Partial<CustomEmoji> = {}): CustomEmoji => ({
  id: '01ABC', server_id: 'S1', name, hash: 'ab'.repeat(32), still_hash: null, animated: false, added_by: 'U1', created_at: '2026-10-01T00:00:00.000Z', ...over,
})

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

describe('standard emoji', () => {
  it('finds by shortcode, including aliases', () => {
    expect(byShortcode('thumbsup')?.u).toBe('👍')
    expect(byShortcode('nope_not_real')).toBeUndefined()
  })

  it('has groups and every entry points at one', () => {
    expect(groups.length).toBeGreaterThan(5)
    expect(byShortcode('thumbsup')!.g).toBeLessThan(groups.length)
  })

  it('maps to Twemoji file names', () => {
    expect(twemojiUrl('❤️')).toBe('/emoji/2764.svg')
    expect(twemojiUrl('👍🏽')).toBe('/emoji/1f44d-1f3fd.svg')
    expect(twemojiUrl('©️')).toBe('/emoji/a9.svg')
    expect(twemojiUrl('🏃🏽‍♀️')).toBe('/emoji/1f3c3-1f3fd-200d-2640-fe0f.svg') // ZWJ sequences keep fe0f
  })

  it('maps keycaps and flags', () => {
    expect(twemojiUrl('#️⃣')).toBe('/emoji/23-20e3.svg') // keycap: fe0f dropped, 20e3 kept
    expect(twemojiUrl('1️⃣')).toBe('/emoji/31-20e3.svg')
    expect(twemojiUrl(canonicalKey('*️⃣'))).toBe('/emoji/2a-20e3.svg') // from the stored key too
    expect(twemojiUrl('🇿🇦')).toBe('/emoji/1f1ff-1f1e6.svg') // regional-indicator pair
    expect(twemojiUrl('🏴󠁧󠁢󠁥󠁮󠁧󠁿')).toBe('/emoji/1f3f4-e0067-e0062-e0065-e006e-e0067-e007f.svg') // tag sequence
  })

  it('applies skin tones where available', () => {
    const wave = byShortcode('wave')!
    expect(withSkin(wave, 0)).toBe(wave.u)
    expect(withSkin(wave, 3)).toBe('👋🏽')
    const heart = byShortcode('heart')!
    expect(withSkin(heart, 3)).toBe(heart.u)
  })
})

describe('search', () => {
  it('puts a custom prefix match first', () => {
    const r = search('pog', [custom('pog')])
    expect(r[0]).toMatchObject({ id: '01ABC', name: 'pog' })
  })

  it('strips colons, matches prefixes before keywords, and caps at 50', () => {
    expect(search(':thumbsup:', [])[0]).toMatchObject({ u: '👍' })
    expect(search('smile', []).length).toBeLessThanOrEqual(50)
    expect(search('', [custom('pog')])).toEqual([])
  })

  it('skips deleted custom emoji', () => {
    expect(search('pog', [custom('pog', { hash: null })])).toEqual([])
  })
})

describe('custom emoji tokens', () => {
  it('builds static and animated tokens', () => {
    expect(customToken(custom('pog'))).toBe('<:pog:01ABC>')
    expect(customToken(custom('spin', { animated: true }))).toBe('<a:spin:01ABC>')
  })

  it('matches only well-formed tokens', () => {
    expect(TOKEN_RE.test('<:pog:01ABC>')).toBe(true)
    expect(TOKEN_RE.test('<a:pog:01ABC>')).toBe(true)
    expect(TOKEN_RE.test('<:p o g:1>')).toBe(false)
  })

  it('keys reactions by unicode or custom id', () => {
    expect(reactionKey(byShortcode('thumbsup')!)).toBe('👍')
    expect(reactionKey(custom('pog'))).toBe('custom:01ABC')
  })

  it('keys drop FE0F, like the server', () => {
    const heart = byShortcode('heart')!
    expect(heart.u).toContain('\uFE0F')
    expect(reactionKey(heart)).toBe('❤')
    expect(reactionKey({ ...heart, u: '❤' })).toBe(reactionKey(heart))
    expect(canonicalKey('🏃🏽‍♀️')).toBe('🏃🏽\u200D♀')
    expect(canonicalKey('custom:01ABC')).toBe('custom:01ABC')
  })

  it('finds the Twemoji file from a canonical key, ZWJ sequences included', () => {
    expect(twemojiUrl(canonicalKey('🏃🏽‍♀️'))).toBe('/emoji/1f3c3-1f3fd-200d-2640-fe0f.svg')
    expect(twemojiUrl(canonicalKey('🏳️‍🌈'))).toBe('/emoji/1f3f3-fe0f-200d-1f308.svg')
  })

  it('every emoji, from its stored key, names a Twemoji SVG that exists', () => {
    // The set the build copies into public/emoji/ (scripts/gen-emoji.mjs).
    const dir = dirname(createRequire(import.meta.url).resolve('@twemoji/svg/package.json'))
    const have = new Set(readdirSync(dir).filter((f) => f.endsWith('.svg')))
    const missing: string[] = []
    for (const e of standard) for (const u of [e.u, ...(e.s ?? [])]) {
      const file = twemojiUrl(canonicalKey(u)).replace('/emoji/', '')
      if (!have.has(file)) missing.push(`${u} -> ${file}`)
    }
    expect(have.size).toBeGreaterThan(3000)
    expect(missing).toEqual([])
  })
})
