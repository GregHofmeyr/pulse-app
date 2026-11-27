// @vitest-environment jsdom
// The card editor's own CSS, compiled (jsdom applies the cascade but doesn't lay out).
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { compile } from 'svelte/compiler'
import { beforeAll, describe, expect, it } from 'vitest'

let css = ''
let src = ''
beforeAll(() => {
  src = readFileSync(resolve(__dirname, 'CardEditor.svelte'), 'utf8')
  css = compile(src, { filename: 'CardEditor.svelte', css: 'external' }).css!.code
})
const rule = (sel: string) => css.match(new RegExp(`(?:^|\\}|\\*/)\\s*${sel}\\s*\\{([^}]*)\\}`))?.[1] ?? null

describe('the card editor CSS', () => {
  it('bio_field_keeps_size_between_raw_and_formatted', () => {
    // both states carry the one class, and the one rule gives it the same fixed min and max height
    expect(src).toMatch(/class="bio-view bio-box"|class="bio-box bio-view"/)
    expect(src).toMatch(/class="bio-box"|class="bio-box bio-edit"/)
    const r = css.match(/\.bio-field[^{}]*\.bio-box[^{}]*\{([^}]*)\}/)?.[1] ?? ''
    expect(r).toMatch(/min-height:\s*calc\(/)
    expect(r).toMatch(/max-height:\s*calc\(/)
    expect(r).toMatch(/overflow-y:\s*auto/)
    expect(r).toMatch(/box-sizing:\s*border-box/)
    expect(r).toMatch(/line-height:\s*20px/)
  })
  it('both_bio_states_have_the_same_padding', () => {
    const r = css.match(/\.bio-field[^{}]*\.bio-box[^{}]*\{([^}]*)\}/)?.[1] ?? ''
    expect(r).toMatch(/padding:\s*8px 40px 8px 12px/)
    // the raw state's own rules don't change it
    expect(css.match(/\.bio-edit[^{}]*\{[^}]*\}/g)!.join('')).not.toMatch(/padding/)
  })
  it('name_clips_with_an_ellipsis', () => {
    expect(css).toMatch(/text-overflow:\s*ellipsis/)
  })
  it('long_inputs_are_held_inside', () => {
    expect(rule('\\.name-in\\.svelte-\\w+')).toMatch(/text-overflow:\s*ellipsis/)
    expect(rule('\\.name-in\\.svelte-\\w+')).toMatch(/white-space:\s*nowrap/)
    expect(css).toMatch(/overflow-wrap:\s*anywhere/)
  })
  it('has_no_colour_of_its_own', () => {
    expect(css.replace(/\/\*[\s\S]*?\*\//g, '').match(/#[0-9a-fA-F]{3,8}\b|\brgba?\(/g) ?? []).toEqual([])
  })
})
