import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { compile } from 'svelte/compiler'
import { describe, expect, it } from 'vitest'

// The download icon on a clip's video (#204): Discord-style, top right, only on hover or focus, and out of
// the way of the player's own controls and the wall card's full-screen button.
const css = (file: string) => compile(readFileSync(resolve(__dirname, file), 'utf8'), { filename: file, css: 'external' }).css!.code
const rule = (code: string, selector: RegExp) => code.match(new RegExp(`${selector.source}\\s*\\{([^}]*)\\}`))?.[1] ?? ''

describe('the clip download icon', () => {
  const own = css('ClipDownload.svelte')

  it('looks like the full-screen button on the wall card and hides until wanted', () => {
    const base = rule(own, /\.clip-dl\.[\w-]+/)
    expect(base).toMatch(/position: ?absolute/)
    expect(base).toMatch(/top: ?8px/)
    expect(base).toMatch(/width: ?28px/)
    expect(base).toMatch(/height: ?28px/)
    expect(base).toMatch(/border-radius: ?var\(--radius-sm\)/)
    expect(base).toMatch(/background: ?rgba\(0, ?0, ?0, ?\.6\)/)
    expect(base).toMatch(/opacity: ?0/)
    expect(rule(own, /\.clip-dl\.[\w-]+:focus-visible/)).toMatch(/opacity: ?1/)
  })

  it('shows while the pointer is over the wall card, left of its full-screen button', () => {
    const card = css('ClipCard.svelte')
    expect(rule(card, /\.card\.[\w-]+:hover \.clip-dl/)).toMatch(/opacity: ?1/)
    expect(readFileSync(resolve(__dirname, 'ClipCard.svelte'), 'utf8')).toContain('<ClipDownload onclick={download} beside />')
    expect(rule(own, /\.clip-dl\.beside\.[\w-]+/)).toMatch(/right: ?40px/) // .fs is 28px wide at right: 8px
  })

  it('shows while the pointer is over the player, and not in full screen', () => {
    const player = css('ClipPlayer.svelte')
    expect(rule(player, /\.clip\.[\w-]+:hover \.clip-dl/)).toMatch(/opacity: ?1/)
    expect(rule(player, /\.clip\.[\w-]+:fullscreen \.clip-dl/)).toMatch(/display: ?none/)
  })
})
