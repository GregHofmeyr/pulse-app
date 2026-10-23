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
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }))
    expect(cb).toHaveBeenCalledTimes(2)
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
})
