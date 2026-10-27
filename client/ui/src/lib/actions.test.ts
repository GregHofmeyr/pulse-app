// @vitest-environment jsdom
import { describe, expect, it, vi } from 'vitest'
import { clickOutside, modal } from './actions'

describe('clickOutside', () => {
  it('fires for presses outside and Escape, not inside', async () => {
    const inside = document.createElement('div')
    const outside = document.createElement('div')
    document.body.append(inside, outside)
    const cb = vi.fn()
    const a = clickOutside(inside, cb)
    await new Promise((r) => setTimeout(r, 0)) // armed after the opening click
    inside.dispatchEvent(new Event('pointerdown', { bubbles: true }))
    expect(cb).not.toHaveBeenCalled()
    outside.dispatchEvent(new Event('pointerdown', { bubbles: true }))
    expect(cb).toHaveBeenCalledTimes(1)
    const esc = new KeyboardEvent('keydown', { key: 'Escape', cancelable: true })
    window.dispatchEvent(esc)
    expect(cb).toHaveBeenCalledTimes(2)
    // consumed: whatever sits under the menu leaves it alone
    expect(esc.defaultPrevented).toBe(true)
    const toggle = document.createElement('button')
    toggle.dataset.menuToggle = ''
    document.body.append(toggle)
    toggle.dispatchEvent(new Event('pointerdown', { bubbles: true }))
    expect(cb).toHaveBeenCalledTimes(2)
    a.destroy()
    outside.dispatchEvent(new Event('pointerdown', { bubbles: true }))
    expect(cb).toHaveBeenCalledTimes(2)
  })
})

describe('modal', () => {
  const setup = () => {
    const opener = document.createElement('button')
    const dialog = document.createElement('div')
    dialog.tabIndex = -1
    dialog.innerHTML = '<button id="a">a</button><input id="b" /><button id="c" disabled>c</button><button id="d">d</button>'
    document.body.append(opener, dialog)
    opener.focus()
    return { opener, dialog }
  }
  const key = (el: Element, k: string, shiftKey = false) => {
    const e = new KeyboardEvent('keydown', { key: k, shiftKey, bubbles: true, cancelable: true })
    el.dispatchEvent(e)
    return e
  }

  it('moves focus in, traps Tab both ways, closes on Escape, restores focus on destroy', () => {
    const { opener, dialog } = setup()
    const onClose = vi.fn()
    const a = modal(dialog, onClose)
    const first = dialog.querySelector<HTMLElement>('#a')!
    const last = dialog.querySelector<HTMLElement>('#d')!
    expect(document.activeElement).toBe(first)
    last.focus()
    expect(key(last, 'Tab').defaultPrevented).toBe(true)
    expect(document.activeElement).toBe(first)
    expect(key(first, 'Tab', true).defaultPrevented).toBe(true)
    expect(document.activeElement).toBe(last)
    dialog.querySelector<HTMLElement>('#b')!.focus()
    expect(key(document.activeElement!, 'Tab').defaultPrevented).toBe(false) // middle: browser moves it
    key(dialog, 'Escape')
    expect(onClose).toHaveBeenCalledTimes(1)
    a.destroy()
    expect(document.activeElement).toBe(opener)
  })

  it('ignores an Escape that something inside already handled', () => {
    const { dialog } = setup()
    const onClose = vi.fn()
    modal(dialog, onClose)
    dialog.querySelector('#b')!.addEventListener('keydown', (e) => e.preventDefault())
    key(dialog.querySelector('#b')!, 'Escape')
    expect(onClose).not.toHaveBeenCalled()
  })

  it('pulls Tab back in when focus sits on the dialog itself', () => {
    const { dialog } = setup()
    modal(dialog, () => {})
    dialog.focus()
    key(dialog, 'Tab')
    expect(document.activeElement).toBe(dialog.querySelector('#a'))
  })

  it('makes everything behind it inert while open, and puts it back on destroy', () => {
    document.body.innerHTML = ''
    document.body.innerHTML =
      '<div id="app"><nav><button id="opener">o</button></nav><main><p id="side">s</p>' +
      '<div id="host"><div id="bd" data-backdrop></div><div id="dlg" tabindex="-1"><button id="in">in</button><div id="pop"><button id="pick">p</button></div></div></div>' +
      '</main><aside id="already" inert></aside></div><div id="toast"></div><div id="tip" role="tooltip"></div>'
    const $ = (id: string) => document.getElementById(id)!
    const inert = (el: Element) => el.hasAttribute('inert')
    $('opener').focus()
    const a = modal($('dlg'), () => {})
    for (const id of ['toast', 'side']) expect(inert($(id)), id).toBe(true)
    expect(inert(document.querySelector('nav')!)).toBe(true)
    // The dialog, its ancestors and anything nested inside it stay usable.
    for (const id of ['app', 'host', 'dlg', 'pop', 'pick', 'tip', 'bd']) expect(inert($(id)), id).toBe(false)
    expect(document.activeElement).toBe($('in'))
    a.destroy()
    for (const id of ['toast', 'side']) expect(inert($(id)), id).toBe(false)
    expect(inert(document.querySelector('nav')!)).toBe(false)
    expect(inert($('already'))).toBe(true) // was inert before: left alone
    expect(document.activeElement).toBe($('opener'))
    document.body.innerHTML = ''
  })

  it('a modal opened over another takes Escape and Tab first', () => {
    const { dialog } = setup()
    const outer = vi.fn()
    const a = modal(dialog, outer)
    const inner = document.createElement('div')
    inner.tabIndex = -1
    inner.innerHTML = '<button id="x">x</button>'
    dialog.append(inner)
    const innerClose = vi.fn()
    const b = modal(inner, innerClose)
    key(inner, 'Escape')
    expect(innerClose).toHaveBeenCalledTimes(1)
    expect(outer).not.toHaveBeenCalled()
    // Tab wraps inside the top one
    key(inner.querySelector('#x')!, 'Tab')
    expect(document.activeElement).toBe(inner.querySelector('#x'))
    b.destroy()
    key(dialog, 'Escape')
    expect(outer).toHaveBeenCalledTimes(1)
    a.destroy()
    document.body.innerHTML = ''
  })

  it('closing a modal twice leaves the others alone', () => {
    const { dialog } = setup()
    const a = modal(dialog, () => {})
    const other = document.createElement('div')
    other.tabIndex = -1
    other.innerHTML = '<button>y</button>'
    document.body.append(other)
    const otherClose = vi.fn()
    const b = modal(other, otherClose)
    a.destroy()
    a.destroy()
    key(other, 'Escape')
    expect(otherClose).toHaveBeenCalledTimes(1)
    b.destroy()
    document.body.innerHTML = ''
  })
})
