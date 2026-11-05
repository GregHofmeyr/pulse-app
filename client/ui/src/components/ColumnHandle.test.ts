// @vitest-environment jsdom
// The drag handle between a side column and the main area (#160).
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import ColumnHandle from './ColumnHandle.svelte'
import type { Side } from '../lib/columns'

let cmp: ReturnType<typeof mount> | undefined
let onResize = vi.fn()
let onReset = vi.fn()

function show(side: Side, width = 264, min = 200, max = 420) {
  onResize = vi.fn()
  onReset = vi.fn()
  cmp = mount(ColumnHandle, { target: document.body, props: { side, width, min, max, controls: 'panel', label: 'Resize the channel list', onResize, onReset } })
  flushSync()
  return document.querySelector<HTMLElement>('[role=separator]')!
}
const pointer = (target: EventTarget, type: string, clientX: number) => {
  target.dispatchEvent(new MouseEvent(type, { bubbles: true, cancelable: true, clientX, button: 0 }))
  flushSync()
}
const key = (el: Element, k: string, shiftKey = false) => {
  const e = new KeyboardEvent('keydown', { key: k, shiftKey, bubbles: true, cancelable: true })
  el.dispatchEvent(e)
  flushSync()
  return e
}

// a frame at once (the coalescing has its own test below); 0: nothing left pending
beforeEach(() => void vi.stubGlobal('requestAnimationFrame', (cb: FrameRequestCallback) => (cb(0), 0)))
afterEach(() => void vi.unstubAllGlobals())

describe('a drag is one update a frame (#173)', () => {
  it('moves within a frame become one resize, with the latest width; release saves at once', () => {
    const frames: FrameRequestCallback[] = []
    vi.stubGlobal('requestAnimationFrame', (cb: FrameRequestCallback) => frames.push(cb))
    vi.stubGlobal('cancelAnimationFrame', () => void frames.splice(0))
    const h = show('left')
    pointer(h, 'pointerdown', 300)
    pointer(window, 'pointermove', 310)
    pointer(window, 'pointermove', 320)
    pointer(window, 'pointermove', 330)
    expect(onResize).not.toHaveBeenCalled()
    expect(frames).toHaveLength(1)
    frames.shift()!(0)
    expect(onResize).toHaveBeenCalledTimes(1)
    expect(onResize).toHaveBeenLastCalledWith(294, false)
    pointer(window, 'pointermove', 340)
    pointer(window, 'pointerup', 340)
    expect(onResize).toHaveBeenLastCalledWith(304, true)
    expect(frames).toHaveLength(0)
  })
})

afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
  document.documentElement.classList.remove('resizing-columns')
})

describe('ColumnHandle', () => {
  it('is a focusable vertical separator that says its width and limits', () => {
    const h = show('left')
    expect(h.tabIndex).toBe(0)
    expect(h.getAttribute('aria-orientation')).toBe('vertical')
    expect(h.getAttribute('aria-label')).toBe('Resize the channel list')
    expect(h.getAttribute('aria-controls')).toBe('panel')
    expect([h.getAttribute('aria-valuenow'), h.getAttribute('aria-valuemin'), h.getAttribute('aria-valuemax')]).toEqual(['264', '200', '420'])
  })

  it('arrow keys resize and save; other keys are left alone', () => {
    const h = show('left')
    expect(key(h, 'ArrowRight').defaultPrevented).toBe(true)
    expect(onResize).toHaveBeenLastCalledWith(280, true)
    key(h, 'End')
    expect(onResize).toHaveBeenLastCalledWith(420, true)
    expect(key(h, 'Tab').defaultPrevented).toBe(false)
    expect(onResize).toHaveBeenCalledTimes(2)
  })

  it('dragging the left handle right widens the left column, live, and saves once on release', () => {
    const h = show('left')
    pointer(h, 'pointerdown', 300)
    expect(h.classList.contains('dragging')).toBe(true)
    expect(document.documentElement.classList.contains('resizing-columns')).toBe(true)
    pointer(window, 'pointermove', 350)
    expect(onResize).toHaveBeenLastCalledWith(314, false)
    pointer(window, 'pointermove', 1000)
    expect(onResize).toHaveBeenLastCalledWith(420, false)
    pointer(window, 'pointerup', 1000)
    expect(onResize).toHaveBeenLastCalledWith(420, true)
    expect(h.classList.contains('dragging')).toBe(false)
    expect(document.documentElement.classList.contains('resizing-columns')).toBe(false)
    // released: moving on does nothing
    pointer(window, 'pointermove', 200)
    expect(onResize).toHaveBeenLastCalledWith(420, true)
  })

  it('dragging the right handle left widens the right column', () => {
    const h = show('right', 240, 180, 480)
    pointer(h, 'pointerdown', 900)
    pointer(window, 'pointermove', 860)
    expect(onResize).toHaveBeenLastCalledWith(280, false)
    pointer(window, 'pointermove', 1100)
    expect(onResize).toHaveBeenLastCalledWith(180, false)
    pointer(window, 'pointerup', 1100)
  })

  it('a click without a move saves nothing (a double-click leaves the default alone)', () => {
    const h = show('left')
    pointer(h, 'pointerdown', 300)
    pointer(window, 'pointerup', 300)
    expect(onResize).not.toHaveBeenCalled()
  })

  it('a double-click resets the column', () => {
    const h = show('left', 300)
    h.dispatchEvent(new MouseEvent('dblclick', { bubbles: true }))
    flushSync()
    expect(onReset).toHaveBeenCalledTimes(1)
  })
})
