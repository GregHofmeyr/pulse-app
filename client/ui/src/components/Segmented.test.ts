// @vitest-environment jsdom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, describe, expect, it, vi } from 'vitest'
import Segmented from './Segmented.svelte'

let cmp: ReturnType<typeof mount> | undefined
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})
const options = ['All', 'Mentions', 'Threads', 'Reminders', 'Scheduled', 'Saved'].map((l) => ({ value: l.toLowerCase(), label: l }))
const seg = () => document.querySelector<HTMLDivElement>('.seg')!

describe('Segmented', () => {
  it('a radiogroup of radios; a click picks', () => {
    const onchange = vi.fn()
    cmp = mount(Segmented, { target: document.body, props: { label: 'Show', options, value: 'all', onchange } })
    flushSync()
    expect(seg().getAttribute('role')).toBe('radiogroup')
    const radios = [...seg().querySelectorAll('[role=radio]')]
    expect(radios.map((r) => r.getAttribute('aria-checked'))).toEqual(['true', 'false', 'false', 'false', 'false', 'false'])
    ;(radios[5] as HTMLButtonElement).click()
    expect(onchange).toHaveBeenCalledWith('saved')
  })

  it('size sm marks the compact row', () => {
    cmp = mount(Segmented, { target: document.body, props: { label: 'Show', options, value: 'all', onchange: () => {}, size: 'sm' } })
    flushSync()
    expect(seg().classList.contains('sm')).toBe(true)
  })

  it('fades the side that has more to scroll to (#454)', () => {
    cmp = mount(Segmented, { target: document.body, props: { label: 'Show', options, value: 'all', onchange: () => {} } })
    flushSync()
    const el = seg()
    Object.defineProperty(el, 'scrollWidth', { configurable: true, value: 500 })
    Object.defineProperty(el, 'clientWidth', { configurable: true, value: 300 })
    el.scrollLeft = 0
    el.dispatchEvent(new Event('scroll'))
    flushSync()
    expect(el.classList.contains('more-start')).toBe(false)
    expect(el.classList.contains('more-end')).toBe(true)
    el.scrollLeft = 200
    el.dispatchEvent(new Event('scroll'))
    flushSync()
    expect(el.classList.contains('more-start')).toBe(true)
    expect(el.classList.contains('more-end')).toBe(false)
  })

  it('a vertical wheel scrolls the overflowing row sideways', () => {
    cmp = mount(Segmented, { target: document.body, props: { label: 'Show', options, value: 'all', onchange: () => {} } })
    flushSync()
    const el = seg()
    Object.defineProperty(el, 'scrollWidth', { configurable: true, value: 500 })
    Object.defineProperty(el, 'clientWidth', { configurable: true, value: 300 })
    const e = new WheelEvent('wheel', { deltaY: 60, cancelable: true })
    el.dispatchEvent(e)
    expect(e.defaultPrevented).toBe(true)
    expect(el.scrollLeft).toBe(60)
  })

  it('scrolls the picked option into view inside the row, not the page', () => {
    const W = 80
    const idx = (el: HTMLElement) => (el.parentElement ? [...el.parentElement.children].indexOf(el) : 0)
    const spies = [
      vi.spyOn(HTMLElement.prototype, 'offsetLeft', 'get').mockImplementation(function (this: HTMLElement) { return idx(this) * W }),
      vi.spyOn(HTMLElement.prototype, 'offsetWidth', 'get').mockReturnValue(W),
      vi.spyOn(Element.prototype, 'clientWidth', 'get').mockReturnValue(300),
    ]
    try {
      cmp = mount(Segmented, { target: document.body, props: { label: 'Show', options, value: 'saved', onchange: () => {} } })
      flushSync()
      expect(seg().scrollLeft).toBe(5 * W + W - 300 + 24)
    } finally {
      for (const s of spies) s.mockRestore()
    }
  })
})
