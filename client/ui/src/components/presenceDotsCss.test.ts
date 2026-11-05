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
