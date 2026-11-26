// @vitest-environment node
// #97 / #663: a narrow clip card keeps its game tag (the header's chips that used to say it are gone), shortened
// with an ellipsis so the row never spills. A container query on the card, so it follows the card's own width.
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { compile } from 'svelte/compiler'
import { describe, expect, it } from 'vitest'

const css = compile(readFileSync(resolve(__dirname, 'ClipItem.svelte'), 'utf8'), { filename: 'ClipItem.svelte', css: 'external', cssHash: () => 's' }).css!.code

describe('clip card in a narrow column', () => {
  it('the card is a size container', () => {
    expect(css).toMatch(/\.clip-item\.s\s*\{[^}]*container-type:\s*inline-size/)
  })
  it('the game tag is never hidden; below 420 px wide it is capped and ellipsized', () => {
    expect(css).not.toMatch(/\.game\.s\s*\{[^}]*display:\s*none/)
    expect(css).toMatch(/\.game\.s\s*\{[^}]*min-width:\s*0[^}]*overflow:\s*hidden[^}]*text-overflow:\s*ellipsis[^}]*white-space:\s*nowrap/)
    expect(css).toMatch(/@container\s*\(max-width:\s*420px\)\s*\{\s*\.game\.s\s*\{\s*max-width:\s*\d+px;?\s*\}/)
  })
  it('a wall card\'s game and poster shrink with an ellipsis instead of spilling', () => {
    const card = compile(readFileSync(resolve(__dirname, 'ClipCard.svelte'), 'utf8'), { filename: 'ClipCard.svelte', css: 'external', cssHash: () => 's' }).css!.code
    expect(card).toMatch(/\.game\.s\s*\{[^}]*min-width:\s*0[^}]*overflow:\s*hidden[^}]*text-overflow:\s*ellipsis/)
    expect(card).toMatch(/\.who\.s\s*\{[^}]*min-width:\s*0/)
  })
})
