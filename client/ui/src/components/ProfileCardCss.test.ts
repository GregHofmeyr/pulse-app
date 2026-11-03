// @vitest-environment jsdom
// The profile card's own CSS, compiled (jsdom applies the cascade but doesn't lay out) (#131).
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { compile } from 'svelte/compiler'
import { beforeAll, describe, expect, it } from 'vitest'

const scope = 'svelte-test'
let css = ''
beforeAll(() => {
  const src = readFileSync(resolve(__dirname, 'ProfileCard.svelte'), 'utf8')
  css = compile(src, { filename: 'ProfileCard.svelte', css: 'external', cssHash: () => scope }).css!.code
  const style = document.createElement('style')
  style.textContent = css
  document.head.append(style)
})

const el = (html: string) => {
  document.body.innerHTML = html.replaceAll('$s', scope)
  return document.body.querySelector<HTMLElement>('[data-probe]')!
}
/** The declarations of the first rule whose selector matches `sel` exactly. */
const rule = (sel: RegExp) => {
  const m = css.match(new RegExp(`(?:^|\\}|\\*/)\\s*${sel.source}\\s*\\{([^}]*)\\}`))
  return m?.[1] ?? null
}

describe('the actions box on your own card', () => {
  it('has no light edge on top of the status row: no box-shadow border between the rows', () => {
    const second = el('<div class="actions $s"><button class="$s"></button><button data-probe class="$s"></button></div>')
    expect(getComputedStyle(second).boxShadow).toMatch(/^(none)?$/)
  })

  it('separates the rows with a faint 1px line inset from both sides, not full width', () => {
    const before = rule(/\.actions\.svelte-test button:where\(\.svelte-test\) \+ button:where\(\.svelte-test\)::before/)
    expect(before).not.toBeNull()
    expect(before).toMatch(/height:\s*1px/)
    expect(before).toMatch(/top:\s*0/)
    expect(before).toMatch(/left:\s*10px/)
    expect(before).toMatch(/right:\s*10px/)
    // the card's own line colour (lib/cardTheme.ts: the island grey on dark themes, a dark one on light, #139)
    expect(before).toMatch(/background:\s*var\(--card-line\)/)
    const second = el('<div class="actions $s"><button class="$s"></button><button data-probe class="$s"></button></div>')
    expect(getComputedStyle(second).position).toBe('relative')
  })
})

describe('the card follows the app theme (#139)', () => {
  it('has no colour of its own in its CSS: every one comes from cardVars or a theme token', () => {
    expect(css.replace(/\/\*[\s\S]*?\*\//g, '').match(/#[0-9a-fA-F]{3,8}\b|\brgba?\(/g) ?? []).toEqual([])
  })
  it('a light card is lifted off the page by the theme shadow', () => {
    expect(rule(/\.card\.svelte-test/)).toMatch(/box-shadow:[^;]*var\(--card-lift\)/)
  })
})

describe('the showcase badges on the card', () => {
  it('are a little smaller than the shared showcase size (about 15%), every part scaled alike', () => {
    const decl = rule(/\.badges\.svelte-test \.badge-icon\.show/)
    expect(decl).not.toBeNull()
    const zoom = Number(decl!.match(/zoom:\s*([\d.]+)/)?.[1])
    expect(zoom).toBeGreaterThanOrEqual(0.8)
    expect(zoom).toBeLessThanOrEqual(0.85)
  })
})
