// @vitest-environment node
// #237: at narrow widths the quick-action bar no longer covers the end of the line above. --bar-room ramps from 0 at
// 1200px to 4px at 1100px and below (no breakpoint, so a resize never makes the list jump); the bar moves down by
// 1.5x it and grouped messages gain it as top padding. Messages and quotes share the bar's rule.
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { compile } from 'svelte/compiler'
import { describe, expect, it } from 'vitest'

const cssOf = (file: string) =>
  compile(readFileSync(resolve(__dirname, file), 'utf8'), { filename: file, css: 'external', cssHash: () => 's' }).css!.code
const message = cssOf('MessageItem.svelte')
const quote = cssOf('QuoteItem.svelte')
const RAMP = /--bar-room:\s*clamp\(0px,\s*calc\(\(1200px - 100vw\) \/ 25\),\s*4px\)/

describe('quick-action bar at narrow widths', () => {
  it('messages and quotes carry the same ramp', () => {
    expect(message).toMatch(new RegExp(String.raw`\.msg\.s\s*\{[^}]*` + RAMP.source))
    expect(quote).toMatch(new RegExp(String.raw`\.quote\.s\s*\{[^}]*` + RAMP.source))
  })

  it('the bar moves down with it, from -10px wide to -4px narrow, the same in both', () => {
    for (const css of [message, quote]) expect(css).toMatch(/\.actions\.s\s*\{[^}]*top:\s*calc\(-10px \+ 1\.5 \* var\(--bar-room\)\)/)
  })

  it('grouped messages gain it as top padding (1px wide, 5px narrow)', () => {
    expect(message).toMatch(/\.msg\.grouped\.s\s*\{[^}]*padding-top:\s*calc\(1px \+ var\(--bar-room\)\)/)
  })

  it('no width breakpoint: a resize across 1100px moves rows smoothly, never in a step', () => {
    for (const css of [message, quote]) expect(css).not.toMatch(/@media[^{]*width/)
  })
})
