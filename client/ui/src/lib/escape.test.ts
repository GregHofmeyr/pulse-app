// @vitest-environment jsdom
import { afterEach, describe, expect, it } from 'vitest'
import { overlayShown } from './escape'

afterEach(() => (document.body.innerHTML = ''))

describe('overlayShown', () => {
  it('sees a dialog, menu, listbox or emoji popover that is showing', () => {
    for (const html of ['<div role="dialog"></div>', '<div role="menu"></div>', '<div role="listbox"></div>', '<div class="emoji-popover"></div>']) {
      document.body.innerHTML = html
      expect(overlayShown(), html).toBe(true)
    }
  })
  it('ignores nothing open, and one inside [hidden]', () => {
    expect(overlayShown()).toBe(false)
    document.body.innerHTML = '<div hidden><div role="dialog"></div></div>'
    expect(overlayShown()).toBe(false)
  })
  it('ignores one hidden by CSS display:none (itself or an ancestor)', () => {
    document.body.innerHTML = '<div role="dialog" style="display:none"></div>'
    expect(overlayShown()).toBe(false)
    document.body.innerHTML = '<div style="display:none"><div role="menu"></div></div>'
    expect(overlayShown()).toBe(false)
  })
})
