// @vitest-environment node
// #139: the unlock toast sits top-left (under the header, clear of the logo and server tabs), with a short confetti
// burst behind it that never plays under the OS reduced-motion setting.
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { compile } from 'svelte/compiler'
import { describe, expect, it } from 'vitest'

const css = compile(readFileSync(resolve(__dirname, 'BadgeToast.svelte'), 'utf8'), { filename: 'BadgeToast.svelte', css: 'external', cssHash: () => 's' }).css!.code
const rule = (sel: string) => css.match(new RegExp(`(?:^|\\n|\\})\\s*${sel.replace(/\./g, '\\.')}\\.s\\s*\\{([^}]*)\\}`))?.[1] ?? ''

describe('badge toast placement and confetti', () => {
  it('the stack is pinned top-left, below the 52 px header', () => {
    const stack = rule('.stack')
    expect(stack).toMatch(/position:\s*fixed/)
    expect(stack).toMatch(/left:\s*18px/)
    expect(stack).toMatch(/top:\s*6\dpx/)
    expect(stack).not.toMatch(/bottom:/)
  })
  it('the confetti sits behind the toast and does not take the pointer', () => {
    expect(rule('.toast')).toMatch(/isolation:\s*isolate/)
    const c = rule('.confetti')
    expect(c).toMatch(/z-index:\s*-1/)
    expect(c).toMatch(/pointer-events:\s*none/)
  })
  it('the burst lasts about a second and plays once', () => {
    const piece = css.match(/\.confetti\.s i[^{]*\{([^}]*)\}/)?.[1] ?? ''
    expect(piece).toMatch(/animation:[^;]*\s(0?\.9|1(\.0+)?)s\b/)
    expect(piece).toMatch(/animation:[^;]*\bforwards\b/)
  })
  it('hidden under reduced motion', () => {
    expect(css).toMatch(/@media\s*\(prefers-reduced-motion:\s*reduce\)\s*\{\s*\.confetti\.s\s*\{\s*display:\s*none;?\s*\}/)
  })
})
