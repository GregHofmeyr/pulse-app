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
beforeAll(() => {
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
})
