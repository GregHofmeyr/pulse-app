// @vitest-environment jsdom
// The editor's layout, from its own compiled CSS (Greg's UI rule: nothing spills or jumps at any window size).
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { compile } from 'svelte/compiler'
import { beforeAll, describe, expect, it } from 'vitest'

const scope = 'svelte-test'
const compiled = (file: string) => compile(readFileSync(resolve(__dirname, file), 'utf8'), { filename: file, css: 'external', cssHash: () => scope }).css!.code
let editor = ''
let menu = ''
let ring = ''
let typing = ''
beforeAll(() => {
  ring = compiled('RingPopover.svelte')
  typing = compiled('TypingSection.svelte')
  editor = compiled('ProfileEditor.svelte')
  menu = compiled('EditorMenu.svelte')
})
const rule = (css: string, sel: string) => css.match(new RegExp(`(?:^|\\}|\\*/)\\s*${sel.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}\\.${scope}\\s*\\{([^}]*)\\}`))?.[1] ?? ''
/** The body of the narrow-window media query. */
const narrow = (css: string) => css.match(/@media\s*\(max-width:\s*960px\)\s*\{([\s\S]*)\}\s*$/)?.[1] ?? ''

describe('the editor layout (#727)', () => {
  it('narrow_window_menu_icons_only_and_no_horizontal_scroll', () => {
    // the labels go under 960px; the icons stay in a fixed slim column
    expect(narrow(menu)).toMatch(/\.label\.svelte-test\s*\{[^}]*display:\s*none/)
    expect(narrow(menu)).toMatch(/nav\.svelte-test\s*\{[^}]*width:\s*56px/)
    // the menu never scrolls sideways
    expect(rule(menu, 'nav')).toMatch(/overflow-x:\s*hidden/)
    // the modal fits the window with its margin on both sides, and the stage gives way before the page does
    const modal = rule(editor, '.modal')
    expect(modal).toMatch(/max-width:\s*calc\(100vw - 2 \* var\(--editor-margin\)\)/)
    expect(modal).toMatch(/max-height:\s*calc\(100vh - 2 \* var\(--editor-margin\)\)/)
    expect(modal).toMatch(/width:\s*min\(1280px,\s*calc\(100vw - 2 \* var\(--editor-margin\)\)\)/)
    expect(modal).toMatch(/height:\s*min\(880px,/)
    expect(modal).toMatch(/overflow:\s*hidden/)
    expect(rule(editor, '.stage')).toMatch(/min-width:\s*0/)
    expect(rule(editor, '.stage')).toMatch(/overflow-x:\s*hidden/)
    expect(rule(editor, '.body')).toMatch(/min-height:\s*0/)
    // no fixed width but the menu's: the menu (200px, or 56px narrow) is the only one in the row
    const widths = [...editor.matchAll(/[^-]width:\s*(\d+)px/g)].map((m) => m[1])
    expect(widths).toEqual([])
  })

  it('the_menu_label_is_cut_not_the_row', () => {
    const label = rule(menu, '.label')
    expect(label).toMatch(/min-width:\s*0/)
    expect(label).toMatch(/text-overflow:\s*ellipsis/)
    expect(label).toMatch(/white-space:\s*nowrap/)
  })

  it('the_badges_panel_is_wider_and_wraps_under_the_card', () => {
    const side = rule(editor, '.side.wide')
    expect(side).toMatch(/flex:\s*2 1 520px/)
    expect(side).toMatch(/max-width:\s*720px/)
    expect(editor).toMatch(/\.stage\[data-section=['"]?badges['"]?\]/)
  })
})

// Ported from ProfileSettingsCss.test.ts when the old page went (#727, task 10): the same never-spill rules, for where
// the ring tiles and the typing row live now.
describe('the Speaking ring popover never spills (#651)', () => {
  it('the_style_grid_and_swatches_give_way_instead_of_spilling', () => {
    expect(rule(ring, '.grid')).toMatch(/display:\s*grid/)
    expect(rule(ring, '.grid')).toMatch(/grid-template-columns:\s*repeat\(auto-fill,\s*minmax\(104px,\s*1fr\)\)/)
    expect(rule(ring, '.grid')).toMatch(/min-width:\s*0/)
    const tile = rule(ring, '.tile')
    expect(tile).toMatch(/min-width:\s*0/)
    expect(tile).toMatch(/overflow:\s*hidden/)
    const label = rule(ring, '.label')
    expect(label).toMatch(/text-overflow:\s*ellipsis/)
    expect(label).toMatch(/white-space:\s*nowrap/)
    expect(rule(ring, '.swatches')).toMatch(/flex-wrap:\s*wrap/)
    // the caption may wrap inside its panel; the stage keeps a fixed box and reserves its ring's outset by margin
    expect(rule(ring, '.big')).toMatch(/max-width:\s*100%/)
    expect(rule(ring, '.ring-stage')).toMatch(/margin:\s*calc\(/)
    expect(rule(ring, '.caption')).toMatch(/max-width:\s*100%/)
  })
})

describe('the typing text row fits a narrow window (#652)', () => {
  /** The declarations of the rule for `sel`, each of its parts scoped (`.a .b` becomes `.a.svelte-test .b.svelte-test`). */
  const scoped = (sel: string) =>
    typing.match(new RegExp(`(?:^|\\n|\\})\\s*${sel.split(' ').map((p) => `${p.replace(/\./g, '\\.')}\\.${scope}`).join(' ')}\\s*\\{([^}]*)\\}`))?.[1] ?? ''

  it('the_row_shrinks_the_box_first_and_a_long_name_and_the_preview_end_in_an_ellipsis', () => {
    expect(scoped('.typing-row')).toMatch(/display:\s*flex/)
    expect(scoped('.typing-row')).toMatch(/min-width:\s*0/)
    const input = typing.match(new RegExp(`\\.typing-row\\.${scope} \\.input:where\\(\\.${scope}\\) \\{([^}]*)\\}`))?.[1]
    expect(input).toMatch(/flex:\s*1 1 0/)
    expect(input).toMatch(/min-width:\s*0/)
    for (const sel of ['.typing-who', '.typing-preview']) {
      expect(scoped(sel), sel).toMatch(/overflow:\s*hidden/)
      expect(scoped(sel), sel).toMatch(/text-overflow:\s*ellipsis/)
      expect(scoped(sel), sel).toMatch(/white-space:\s*nowrap/)
    }
    expect(scoped('.chips')).toMatch(/flex-wrap:\s*wrap/)
  })
})

