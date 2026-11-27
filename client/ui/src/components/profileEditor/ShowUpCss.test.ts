// @vitest-environment jsdom
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { compile } from 'svelte/compiler'
import { beforeAll, describe, expect, it } from 'vitest'

let css = ''
let src = ''
beforeAll(() => {
  src = readFileSync(resolve(__dirname, 'ShowUp.svelte'), 'utf8')
  css = compile(src, { filename: 'ShowUp.svelte', css: 'external' }).css!.code
})
const rule = (sel: string) => css.match(new RegExp(`(?:^|\\}|\\*/)\\s*${sel}\\s*\\{([^}]*)\\}`))?.[1] ?? null

describe('the where-you-show-up CSS', () => {
  it('width_matches_the_card_and_no_labels_or_boxes', () => {
    const r = rule('\\.show-up\\.svelte-\\w+')!
    expect(r).toMatch(/width:\s*min\(100%,\s*calc\(300px \* var\(--k\)\)\)/)
    expect(r).toMatch(/--k:\s*1\.12/)
    expect(r.replace('box-sizing: border-box', '')).not.toMatch(/border|background|box-shadow|outline/)
    const markup = src.split('<style>')[0]
    expect(markup).not.toMatch(/<h[1-6][\s>]/)
  })
  it('long_names_clip_and_nothing_animates', () => {
    expect(css).toMatch(/text-overflow:\s*ellipsis/)
    // glows are not cut at the sides: clip with a margin, never hidden
    expect(css).not.toMatch(/overflow:\s*hidden/)
    expect(css).toMatch(/overflow:\s*clip/)
    expect(css).toMatch(/overflow-clip-margin:/)
    expect(css).not.toMatch(/animation|transition/)
    expect(css).not.toMatch(/#[0-9a-f]{3,8}\b|rgb\(/i)
  })
})
