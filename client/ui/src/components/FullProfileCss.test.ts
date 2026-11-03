// @vitest-environment jsdom
// The full profile's right column, from its own compiled CSS (#131): its boards (the trophy case is the
// tall one) scroll under the column's edge, and that edge is cut with the boards' own round corners, so
// a board scrolled half out of view keeps rounded corners instead of a square cut.
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { compile } from 'svelte/compiler'
import { beforeAll, describe, expect, it } from 'vitest'

const scope = 'svelte-test'
let css = ''
beforeAll(() => {
  const src = readFileSync(resolve(__dirname, 'FullProfile.svelte'), 'utf8')
  css = compile(src, { filename: 'FullProfile.svelte', css: 'external', cssHash: () => scope }).css!.code
})
/** The declarations of the first top-level rule for `sel`. */
const rule = (sel: string) => {
  const m = css.match(new RegExp(`(?:^|\\}|\\*/)\\s*${sel.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}\\s*\\{([^}]*)\\}`))
  return m?.[1] ?? null
}

describe('the full profile right column', () => {
  it('rounds the scroll edge with the board radius, at the boards, not past the scrollbar', () => {
    const board = rule(`.board.${scope}`)!
    const radius = board.match(/border-radius:\s*(\d+px)/)![1]
    expect(rule(`.right.${scope}`)).toMatch(/overflow-y:\s*auto/)
    // only where the gutter can be kept (older WebKit: no mask, rather than a cut past the boards)
    const supports = css.slice(css.indexOf('@supports (scrollbar-gutter: stable)'))
    expect(supports.length).toBeLessThan(css.length)
    const right = supports.match(new RegExp(`\\.right\\.${scope}\\s*\\{([^}]*)\\}`))![1]
    // the scrollbar's room is always there, so the boards' right edge sits at a fixed place
    expect(right).toMatch(/scrollbar-gutter:\s*stable/)
    // a mask: a round corner of the board radius in each corner of the boards' area, plus the scrollbar strip
    expect(right).toMatch(/\bmask:/)
    const corners = right.match(/radial-gradient\(circle (\d+px)/g) ?? []
    expect(corners).toHaveLength(4)
    expect(corners.every((c) => c.endsWith(radius))).toBe(true)
  })

  it('drops the mask where the column stops scrolling (narrow: the whole dialog scrolls)', () => {
    const narrow = css.slice(css.indexOf('@media (max-width: 760px)'))
    expect(narrow).toMatch(/\.right\.svelte-test\s*\{[^}]*mask:\s*none/)
  })
})
