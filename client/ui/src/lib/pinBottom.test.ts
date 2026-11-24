// @vitest-environment jsdom
// Staying at the bottom (#292) without fighting the user (#613): scrolling up never re-pins a list, whatever a layout
// change under the view does to its distance from the bottom; only coming back down (or reaching it) does.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { followsBottom, pinToBottom } from './pinBottom'

describe('followsBottom', () => {
  it('the first measure is by distance alone', () => {
    expect(followsBottom(false, null, 20, 40)).toBe(true)
    expect(followsBottom(true, null, 60, 40)).toBe(false)
  })
  it('scrolling up keeps a pinned list pinned only within the threshold, and never pins one', () => {
    expect(followsBottom(true, -8, 8, 40)).toBe(true)
    expect(followsBottom(true, -8, 40, 40)).toBe(false)
    // a layout change brought the bottom closer as the view moved up: still not pinned
    expect(followsBottom(false, -8, 29, 40)).toBe(false)
  })
  it('scrolling down pins within the threshold', () => {
    expect(followsBottom(false, 8, 39, 40)).toBe(true)
    expect(followsBottom(false, 8, 120, 40)).toBe(false)
  })
  it('at the very bottom it is pinned however it got there (the list got shorter under it)', () => {
    expect(followsBottom(false, -300, 0, 40)).toBe(true)
    expect(followsBottom(false, -300, 0.5, 40)).toBe(true) // fractional scrollTop at a display scale
  })
  it('no movement: as it was', () => {
    expect(followsBottom(true, 0, 30, 40)).toBe(true)
    expect(followsBottom(false, 0, 30, 40)).toBe(false)
  })
})

describe('pinToBottom', () => {
  let cb: ResizeObserverCallback | undefined
  let el: HTMLElement
  let top = 0
  let sh = 2000
  let ch = 500
  beforeEach(() => {
    cb = undefined
    vi.stubGlobal('ResizeObserver', class {
      constructor(c: ResizeObserverCallback) { cb = c }
      observe() {}
      unobserve() {}
      disconnect() {}
    })
    el = document.createElement('div')
    document.body.append(el)
    sh = 2000
    ch = 500
    top = sh - ch
    Object.defineProperties(el, {
      scrollHeight: { get: () => sh },
      clientHeight: { get: () => ch },
      scrollTop: { get: () => top, set: (v: number) => (top = Math.max(0, Math.min(v, sh - ch))) },
    })
  })
  afterEach(() => {
    el.remove()
    vi.unstubAllGlobals()
  })
  const scrollTo = (t: number) => {
    top = t
    el.dispatchEvent(new Event('scroll'))
  }
  const resize = () => cb?.([], {} as ResizeObserver)

  it('a smooth scroll up past the threshold is not pulled back, even when the list grows taller as it leaves', () => {
    const seen: boolean[] = []
    // what a chat does as it leaves the bottom: a Jump button shows and the list ends up 11px taller (0.2.19, #613)
    const p = pinToBottom(el, { on: () => true, onScroll: (pinned) => { seen.push(pinned); if (!pinned) ch = 511 } })
    for (let up = 8; up <= 64; up += 8) {
      const was = ch
      scrollTo(2000 - 500 - up)
      if (ch !== was) resize() // the observer runs when the list's size changed
      expect(top).toBe(2000 - 500 - up)
    }
    expect(p.pinned()).toBe(false)
    expect(seen.at(-1)).toBe(false)
    p.destroy()
  })

  it('coming back down within the threshold pins it again, and growth then keeps it at the bottom', () => {
    const p = pinToBottom(el, { on: () => true })
    scrollTo(1000)
    expect(p.pinned()).toBe(false)
    scrollTo(1480)
    expect(p.pinned()).toBe(true)
    sh = 2200
    resize()
    expect(top).toBe(sh - ch)
    p.destroy()
  })

  it('tells its caller the same answer it acts on, on every scroll event', () => {
    const seen: boolean[] = []
    const p = pinToBottom(el, { on: () => true, onScroll: (pinned) => seen.push(pinned) })
    scrollTo(1490)
    scrollTo(1400)
    scrollTo(1470)
    expect(seen).toEqual([true, false, true])
    p.destroy()
  })
})
