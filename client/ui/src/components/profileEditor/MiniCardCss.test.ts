// @vitest-environment jsdom
// The miniature card's own CSS, compiled: the theme pills were empty because these rules were never written.
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { compile } from 'svelte/compiler'
import { beforeAll, describe, expect, it } from 'vitest'

let css = ''
let src = ''
beforeAll(() => {
  src = readFileSync(resolve(__dirname, 'MiniCard.svelte'), 'utf8')
  css = compile(src, { filename: 'MiniCard.svelte', css: 'external', cssHash: () => 'x' }).css!.code
})
const rule = (sel: string) => css.match(new RegExp(`(?:^|\\}|\\*/)\\s*\\${sel}\\.x\\s*\\{([^}]*)\\}`))?.[1] ?? ''

describe('MiniCard CSS', () => {
  it('mini_card_draws_banner_avatar_and_lines', () => {
    expect(src).toMatch(/mc-banner/)
    expect(rule('.mc-banner')).toMatch(/height:\s*\d+%?px?|height:\s*\d+%/)
    expect(rule('.mc-banner')).toMatch(/var\(--card-border-top\)|var\(--card-top\)/)
    expect(rule('.mc-avatar')).toMatch(/width:\s*\d+px/)
    expect(rule('.mc-avatar')).toMatch(/height:\s*\d+px/)
    expect(rule('.mc-avatar')).toMatch(/border-radius:\s*50%/)
    expect(rule('.mc-avatar')).toMatch(/var\(--card-text\)/)
    expect(rule('.mc-line')).toMatch(/height:\s*\d+px/)
    expect(rule('.mc-line')).toMatch(/var\(--card-text-2\)|var\(--card-skeleton\)/)
    expect(css).toMatch(/\.mc-line\.short\.x/)
    // the card itself is the theme's gradient, top to bottom
    expect(rule('.mc')).toMatch(/linear-gradient\(to bottom,\s*var\(--card-top\),\s*var\(--card-bottom\)\)/)
    expect(rule('.mc')).toMatch(/overflow:\s*hidden/)
  })
})
