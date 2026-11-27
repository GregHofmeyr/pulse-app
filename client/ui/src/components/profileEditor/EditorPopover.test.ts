// @vitest-environment jsdom
import { flushSync, mount, unmount, createRawSnippet } from 'svelte'
import { afterEach, describe, expect, it, vi } from 'vitest'
import EditorPopover from './EditorPopover.svelte'

let cmp: ReturnType<typeof mount> | undefined
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})
const snip = (t: string) => createRawSnippet(() => ({ render: () => `<p>${t}</p>` }))
function show() {
  const onCancel = vi.fn()
  const onApply = vi.fn()
  cmp = mount(EditorPopover, { target: document.body, props: { title: 'Thing', onCancel, onApply, tiles: snip('tiles'), preview: snip('preview') } })
  flushSync()
  return { onCancel, onApply }
}

describe('EditorPopover', () => {
  it('is_a_labelled_editor_layer_with_tiles_and_preview', () => {
    show()
    const d = document.querySelector('[role=dialog]')!
    expect(d.getAttribute('aria-label')).toBe('Thing')
    expect(d.hasAttribute('data-editor-layer')).toBe(true)
    expect(d.textContent).toContain('tiles')
    expect(d.textContent).toContain('preview')
  })
  it('escape_cancels_and_is_marked_handled', () => {
    const { onCancel } = show()
    const e = new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true })
    window.dispatchEvent(e)
    expect(e.defaultPrevented).toBe(true)
    expect(onCancel).toHaveBeenCalledTimes(1)
  })
  it('buttons_and_scrim', () => {
    const { onCancel, onApply } = show()
    ;(document.querySelector('button[aria-label="Apply"]') as HTMLElement).click()
    expect(onApply).toHaveBeenCalledTimes(1)
    ;(document.querySelector('.scrim') as HTMLElement).click()
    expect(onCancel).toHaveBeenCalledTimes(1)
  })
  it('tab_wraps_inside_the_popover', () => {
    show()
    // the dialog's own first control sits behind, inert
    const behind = document.createElement('button')
    behind.textContent = 'x'
    document.body.prepend(behind)
    const list = [...document.querySelectorAll<HTMLElement>('[data-editor-layer] button:not(.scrim)')]
    const first = list[0]
    const last = list[list.length - 1]
    expect(first).not.toBe(last)
    const tab = (shiftKey: boolean) => {
      const e = new KeyboardEvent('keydown', { key: 'Tab', shiftKey, bubbles: true, cancelable: true })
      window.dispatchEvent(e)
      return e
    }
    last.focus()
    expect(tab(false).defaultPrevented).toBe(true)
    expect(document.activeElement).toBe(first)
    expect(tab(true).defaultPrevented).toBe(true)
    expect(document.activeElement).toBe(last)
    // between the ends Tab is left to the browser
    const mid = document.createElement('input')
    first.after(mid)
    mid.focus()
    expect(tab(false).defaultPrevented).toBe(false)
  })
})
