// @vitest-environment jsdom
import { describe, expect, it, vi } from 'vitest'
import { clickOutside } from './actions'

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
