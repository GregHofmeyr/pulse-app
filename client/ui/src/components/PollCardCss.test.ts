// @vitest-environment node
// #157: a chosen option's tick sits in the middle of its mark. The mark is border-box, so its inside (the size less
// the border on both sides) has to hold the 14 px tick, or the tick spills out to the right.
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { compile } from 'svelte/compiler'
import { describe, expect, it } from 'vitest'

const src = readFileSync(resolve(__dirname, 'PollCard.svelte'), 'utf8')
const css = compile(src, { filename: 'PollCard.svelte', css: 'external', cssHash: () => 's' }).css!.code
const rule = (sel: string) => css.match(new RegExp(`(?:^|\\n|\\})\\s*${sel.replace(/\./g, '\\.')}\\.s\\s*\\{([^}]*)\\}`))?.[1] ?? ''
const px = (decl: string, prop: string) => Number(decl.match(new RegExp(`(?:^|;|\\s)${prop}:\\s*(\\d+)px`))?.[1] ?? NaN)

describe('the poll option mark', () => {
  it('holds the 14 px tick inside its border, centred', () => {
    const mark = rule('.mark')
    const tick = Number(src.match(/class="mark"[^\n]*<Icon name="check" size=\{(\d+)\}/)?.[1])
    expect(tick).toBe(14)
    const w = px(mark, 'width')
    const h = px(mark, 'height')
    const border = Number(mark.match(/border:\s*(\d+)px/)?.[1])
    expect(w).toBe(h)
    expect(w - 2 * border).toBeGreaterThanOrEqual(tick)
    expect(mark).toMatch(/place-items:\s*center/)
  })
})
