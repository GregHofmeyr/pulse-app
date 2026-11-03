// @vitest-environment jsdom
// A thin gap between the @ and the name in mention pills and composer / edit chips (#137): the @ is its own
// `.at` span (lib/mentionLabel.ts), and the CSS that draws pills and chips spaces it from the name.
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { compile } from 'svelte/compiler'
import { describe, expect, it } from 'vitest'

const cssOf = (file: string) => compile(readFileSync(resolve(__dirname, file), 'utf8'), { filename: file, css: 'external', cssHash: () => 's' }).css!.code

/** The em gap a rule matching `sel` sets after the @, or null. */
function gap(css: string, sel: RegExp): number | null {
  const m = css.match(new RegExp(`${sel.source}\\s*\\{([^}]*)\\}`))
  const v = m?.[1].match(/margin-right:\s*([\d.]+)em/)?.[1]
  return v === undefined ? null : Number(v)
}

describe('the @ in a mention (#137)', () => {
  it('message pills: a thin gap after the @', () => {
    const g = gap(cssOf('MessageItem.svelte'), /\.content\.s \.mention \.at/)
    expect(g).not.toBeNull()
    expect(g!).toBeGreaterThanOrEqual(0.1)
    expect(g!).toBeLessThanOrEqual(0.25)
  })

  it('composer and edit chips: the same gap', () => {
    const g = gap(cssOf('EmojiBox.svelte'), /\.box\.s \.mention-chip \.at/)
    expect(g).not.toBeNull()
    expect(g!).toBeGreaterThanOrEqual(0.1)
    expect(g!).toBeLessThanOrEqual(0.25)
  })
})
