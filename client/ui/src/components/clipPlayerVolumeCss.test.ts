import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { compile } from 'svelte/compiler'
import { describe, expect, it } from 'vitest'

// The compact (clips wall) player's volume (#208): the speaker sits in the bar, and its vertical slider pops up
// above it on hover or keyboard focus, over the video, clear of the download icon (top right) and the bar's
// full-screen button.
const code = compile(readFileSync(resolve(__dirname, 'ClipPlayer.svelte'), 'utf8'), { filename: 'ClipPlayer.svelte', css: 'external' }).css!.code
const rule = (selector: RegExp) => code.match(new RegExp(`${selector.source}\\s*\\{([^}]*)\\}`))?.[1] ?? ''

describe('the compact player volume pop-up', () => {
  it('sits above the speaker, hidden until wanted', () => {
    const pop = rule(/\.vol-pop\.[\w-]+/)
    expect(pop).toMatch(/position: ?absolute/)
    expect(pop).toMatch(/bottom: ?100%/)
    expect(pop).toMatch(/visibility: ?hidden/)
    expect(pop).toMatch(/opacity: ?0/)
  })

  it('shows on hover and on keyboard focus', () => {
    const shown = code.match(/([^{}]*)\{[^}]*visibility: ?visible[^}]*\}/)?.[1] ?? ''
    expect(shown).toMatch(/\.vol-wrap\.[\w-]+:hover \.vol-pop/)
    expect(shown).toMatch(/\.vol-wrap\.[\w-]+:focus-within \.vol-pop/)
  })

  it('stands the slider upright and stays short enough to clear the download icon', () => {
    expect(rule(/\.vol-pop\.[\w-]+ input(?:\.[\w-]+|:where\(\.[\w-]+\))/)).toMatch(/rotate\(-90deg\)/)
    const h = Number(rule(/\.vol-pop\.[\w-]+/).match(/height: ?(\d+)px/)?.[1])
    // a 260px-wide wall card has a ~146px screen; the download icon takes its top 36px
    expect(h).toBeGreaterThan(0)
    expect(h).toBeLessThanOrEqual(146 - 36 - 8)
  })
})
