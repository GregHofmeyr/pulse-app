// @vitest-environment node
// #454: nothing spills past its box. The inbox's tab row scrolls inside the popover; dialog and popover button rows
// wrap instead of pushing a button off the edge; a floating confirm and a toast stay inside the window.
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { compile } from 'svelte/compiler'
import { describe, expect, it } from 'vitest'

const cssOf = (file: string) =>
  compile(readFileSync(resolve(__dirname, file), 'utf8'), { filename: file, css: 'external', cssHash: () => 's' }).css!.code
const rule = (css: string, sel: string) => {
  const m = css.match(new RegExp(String.raw`(?:^|[}/])\s*` + sel.replace(/[.[\]():]/g, (c) => '\\' + c) + String.raw`\s*\{([^}]*)\}`))
  if (!m) throw new Error(`no ${sel} rule`)
  return m[1]
}

describe('overflow (#454)', () => {
  it('a segmented row is never wider than its box: it scrolls sideways, its options keep their one line', () => {
    const css = cssOf('Segmented.svelte')
    expect(rule(css, '.seg.s')).toMatch(/max-width:\s*100%/)
    expect(rule(css, '.seg.s')).toMatch(/overflow-x:\s*auto/)
    expect(rule(css, '.seg.s button:where(.s)')).toMatch(/flex:\s*none/)
    expect(rule(css, '.seg.s button:where(.s)')).toMatch(/white-space:\s*nowrap/)
  })

  it('the side with more is faded', () => {
    const css = cssOf('Segmented.svelte')
    expect(rule(css, '.seg.s')).toMatch(/mask-image:\s*linear-gradient/)
    expect(rule(css, '.seg.more-start.s')).toMatch(/--fade-start:\s*28px/)
    expect(rule(css, '.seg.more-end.s')).toMatch(/--fade-end:\s*28px/)
  })

  it("the inbox's tab row can shrink to the popover's width (a flex child needs min-width: 0)", () => {
    expect(rule(cssOf('InboxPopover.svelte'), '.tabs.s')).toMatch(/min-width:\s*0/)
  })

  const rows: [string, string][] = [
    ['ClipActions.svelte', '.foot.s'], ['ChannelSettingsDialog.svelte', '.actions.s'], ['DeleteChannelDialog.svelte', '.actions.s'],
    ['NicknameDialog.svelte', '.actions.s'], ['ForwardPicker.svelte', '.foot.s'], ['SoundDialog.svelte', '.actions.s'],
    ['StreamClashDialog.svelte', '.actions.s'], ['ImageEditDialog.svelte', '.actions.s'], ['GoLiveDialog.svelte', '.actions.s'],
    ['WhenPicker.svelte', '.foot.s'], ['CustomStatusDialog.svelte', '.actions.s'], ['QotyStart.svelte', '.actions.s'],
    ['ServerActionDialog.svelte', '.actions.s'], ['CropDialog.svelte', '.actions.s'], ['SaveWheelForm.svelte', '.btns.s'], ['Soundboard.svelte', '.btns.s'],
  ]
  for (const [file, sel] of rows) {
    it(`${file}: its button row wraps (a button is one line, so a narrow box would push one off the edge)`, () => {
      expect(rule(cssOf(file), sel)).toMatch(/flex-wrap:\s*wrap/)
    })
  }

  it('the delete confirm is capped to the window and scrolls inside', () => {
    const ask = rule(cssOf('DeleteButton.svelte'), '.ask.s')
    expect(ask).toMatch(/max-width:\s*calc\(100vw - 16px\)/)
    expect(ask).toMatch(/overflow-y:\s*auto/)
  })

  it('a toast is never wider than the window, and long words break', () => {
    const toast = rule(readFileSync(resolve(__dirname, '../app.css'), 'utf8'), '.toast')
    expect(toast).toMatch(/max-width:\s*calc\(100vw - 16px\)/)
    expect(toast).toMatch(/overflow-wrap:\s*anywhere/)
  })
})
