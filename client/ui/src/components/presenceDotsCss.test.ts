// @vitest-environment jsdom
// Your own Invisible dot is the plain grey offline dot (#171): the owner found a hollow one inconsistent. The status
// picker and the line under your name still say Invisible. Each place that draws an Invisible dot is checked, compiled.
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { compile } from 'svelte/compiler'
import { describe, expect, it } from 'vitest'

const cssOf = (file: string) =>
  compile(readFileSync(resolve(__dirname, file), 'utf8'), { filename: file, css: 'external', cssHash: () => 's' }).css!.code

/** The declarations of every rule whose selector list includes `sel`, joined. */
const decls = (css: string, sel: string) =>
  [...css.replace(/\/\*[\s\S]*?\*\//g, '').matchAll(/([^{}]+)\{([^}]*)\}/g)]
    .filter(([, sels]) => sels.split(',').some((s) => s.trim().replace(/\.s\b/g, '') === sel))
    .map(([, , d]) => d)
    .join(';')

describe('the Invisible dot looks like the offline dot (#171)', () => {
  it.each([
    ['Avatar.svelte', '.pres.invisible', '.pres.offline'],
    ['StatusPicker.svelte', '.dot.invisible', null],
    ['ProfileCard.svelte', '.status.invisible', null],
  ])('%s', (file, invisible, offline) => {
    const css = cssOf(file)
    expect(css).not.toMatch(/radial-gradient/)
    const inv = decls(css, invisible)
    expect(inv).toMatch(/background:\s*var\(--presence-off\)/)
    if (offline) expect(decls(css, offline)).toMatch(/background:\s*var\(--presence-off\)/)
  })
})

describe('offline members are dimmed slightly (#170)', () => {
  it.each(['MemberList.svelte', 'GroupMembers.svelte'])('%s', (file) => {
    const m = decls(cssOf(file), '.row.off').match(/opacity:\s*([\d.]+)/)
    expect(m).not.toBeNull()
    expect(Number(m![1])).toBeGreaterThanOrEqual(0.4)
    expect(Number(m![1])).toBeLessThan(1)
  })
})

describe('member list dots are small and cut out of the avatar (#169)', () => {
  it.each(['MemberList.svelte', 'GroupMembers.svelte'])('%s', (file) => {
    const css = cssOf(file)
    // smaller than the shared dot (34% of the avatar, at least 9 px)
    const dot = decls(css, '.row .pres')
    expect(dot).toMatch(/width:\s*12px/)
    expect(dot).toMatch(/height:\s*12px/)
    // the cut-out ring is the row's own colour, also while hovered
    const hover = decls(css, '.row:hover')
    expect(hover).toMatch(/background:\s*var\(--bg-2\)/)
    expect(hover).toMatch(/--ring:\s*var\(--bg-2\)/)
  })
})
