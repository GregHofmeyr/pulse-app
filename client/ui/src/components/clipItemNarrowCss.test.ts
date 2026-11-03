// @vitest-environment node
// #97: a narrow clip card drops its game tag (the feed's chips still say which game). A container query on the
// card, so it follows the card's own width (side panels open), not the window's.
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { compile } from 'svelte/compiler'
import { describe, expect, it } from 'vitest'

const css = compile(readFileSync(resolve(__dirname, 'ClipItem.svelte'), 'utf8'), { filename: 'ClipItem.svelte', css: 'external', cssHash: () => 's' }).css!.code

describe('clip card in a narrow column', () => {
  it('the card is a size container', () => {
    expect(css).toMatch(/\.clip-item\.s\s*\{[^}]*container-type:\s*inline-size/)
  })
  it('below 420 px wide the game tag is hidden', () => {
    expect(css).toMatch(/@container\s*\(max-width:\s*420px\)\s*\{\s*\.game\.s\s*\{\s*display:\s*none;?\s*\}/)
  })
})
