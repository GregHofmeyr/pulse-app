// @vitest-environment jsdom
// The "Speaking ring" tab (#651 §7.1), from its own compiled CSS: nothing spills out of its container or jumps at any
// width (Greg's UI rule). The style chips ellipsise inside their grid cells, the swatches wrap, and the preview panel
// never shrinks: when there's no room beside it, the row stacks instead.
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { compile } from 'svelte/compiler'
import { beforeAll, describe, expect, it } from 'vitest'
import { PREVIEW_BOX, PREVIEW_D } from '../lib/speakingRing'

const scope = 'svelte-test'
let css = ''
beforeAll(() => {
  const src = readFileSync(resolve(__dirname, 'ProfileSettings.svelte'), 'utf8')
  css = compile(src, { filename: 'ProfileSettings.svelte', css: 'external', cssHash: () => scope }).css!.code
})
/** The declarations of the first top-level rule for `sel` (one class). */
const rule = (sel: string) => {
  const m = css.match(new RegExp(`(?:^|\\}|\\*/)\\s*${sel.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}\\.${scope}\\s*\\{([^}]*)\\}`))
  return m?.[1] ?? ''
}

describe('the speaking ring tab never spills (#651)', () => {
  it('the_style_grid_never_spills', () => {
    expect(rule('.ring-grid')).toMatch(/display:\s*grid/)
    expect(rule('.ring-grid')).toMatch(/grid-template-columns:\s*repeat\(3,\s*minmax\(0,\s*1fr\)\)/)
    const chip = rule('.ring-chip')
    expect(chip).toMatch(/min-width:\s*0/)
    expect(chip).toMatch(/overflow:\s*hidden/)
    expect(chip).toMatch(/text-overflow:\s*ellipsis/)
    expect(chip).toMatch(/white-space:\s*nowrap/)
    // the controls column can shrink below its content, so the grid (not the page) gives way
    expect(rule('.ring-controls')).toMatch(/min-width:\s*0/)
  })

  it('the_preview_never_shrinks_and_the_row_stacks_instead', () => {
    expect(rule('.ring-row')).toMatch(/display:\s*flex/)
    expect(rule('.ring-row')).toMatch(/flex-wrap:\s*wrap/)
    const preview = rule('.ring-preview')
    expect(preview).toMatch(new RegExp(`flex:\\s*0 0 ${PREVIEW_BOX}px`))
    expect(preview).toMatch(new RegExp(`(?:^|;|\\s)width:\\s*${PREVIEW_BOX}px`))
    expect(preview).toMatch(new RegExp(`(?:^|;|\\s)height:\\s*${PREVIEW_BOX}px`))
    expect(rule('.ring-swatches')).toMatch(/flex-wrap:\s*wrap/)
    // the 96px avatar's 1.32 D ring box fits the panel, and its outset is reserved by margin
    expect(PREVIEW_D).toBeLessThanOrEqual(PREVIEW_BOX / 1.32)
    expect(rule('.ring-stage')).toMatch(new RegExp(`width:\\s*${PREVIEW_D}px`))
    expect(rule('.ring-stage')).toMatch(/margin:\s*calc\(/)
    // the caption may wrap inside the panel, never past it
    expect(rule('.ring-caption')).toMatch(/overflow-wrap:\s*anywhere/)
    // tokens only: the panel's surface, border and radius
    expect(preview).toMatch(/background:\s*var\(--bg-1\)/)
    expect(preview).toMatch(/border:\s*1px solid var\(--bg-3\)/)
    expect(preview).toMatch(/border-radius:\s*var\(--radius-xl\)/)
  })
})
