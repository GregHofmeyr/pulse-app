// @vitest-environment jsdom
// Drag to reorder (#55): the slot and shift maths the sortable list runs every frame, and how a drop that changes
// nothing ends (the whole drag is covered through ChannelList in ChannelListDrag.test.ts).
import { afterEach, describe, expect, it, vi } from 'vitest'
import { EDGE_MAX_SPEED, EDGE_ZONE, easeSpeed, edgeSpeed, shiftsFor, slotFor, sortableList } from './drag'
import { motion } from './animate.svelte'

describe('slotFor', () => {
  const items = [{ top: 0, height: 34 }, { top: 34, height: 34 }, { top: 68, height: 30 }]

  it('counts the items whose middle is above the pointer', () => {
    expect(slotFor(items, -10)).toBe(0)
    expect(slotFor(items, 16)).toBe(0)
    expect(slotFor(items, 18)).toBe(1)
    expect(slotFor(items, 50)).toBe(1)
    expect(slotFor(items, 52)).toBe(2)
    expect(slotFor(items, 84)).toBe(3)
    expect(slotFor(items, 500)).toBe(3)
  })

  it('an empty list has one slot', () => {
    expect(slotFor([], 40)).toBe(0)
  })
})

describe('shiftsFor', () => {
  it('nothing moves while the item is over its own slot', () => {
    expect(shiftsFor(1, 1, 34, 4)).toEqual([0, 0, 0, 0])
  })

  it('moving down: the items it passes move up by its height', () => {
    // item 0 to after item 2 (slot 2 among the other three)
    expect(shiftsFor(0, 2, 34, 4)).toEqual([0, -34, -34, 0])
  })

  it('moving up: the items it passes move down by its height', () => {
    // item 3 to the very top
    expect(shiftsFor(3, 0, 34, 4)).toEqual([34, 34, 34, 0])
  })

  it('coming from another list (fromIndex -1): the items after the slot make room', () => {
    expect(shiftsFor(-1, 1, 30, 3)).toEqual([0, 30, 30])
  })

  it('leaving for another list (toSlot -1): the items after it close the gap', () => {
    expect(shiftsFor(1, -1, 30, 3)).toEqual([0, 0, -30])
  })

  it('the lifted item itself never gets a shift (it follows the pointer)', () => {
    expect(shiftsFor(2, 0, 34, 3)[2]).toBe(0)
  })
})

describe('sortableList: a drop that changes nothing', () => {
  // two rows, 34px each, from the list's top at 0 (jsdom lays nothing out)
  const ROW = 34
  function setup(onDrop: () => boolean) {
    document.body.innerHTML = '<div class="list"><div class="cat top"><div class="row" data-channel="A"></div><div class="row" data-channel="B"></div></div></div>'
    const list = document.querySelector<HTMLElement>('.list')!
    const rows = [...list.querySelectorAll<HTMLElement>('.row')]
    const ty = (el: HTMLElement) => Number(/, (-?[\d.]+)px/.exec(el.style.transform)?.[1] ?? 0)
    vi.spyOn(Element.prototype, 'getBoundingClientRect').mockImplementation(function (this: Element) {
      const i = rows.indexOf(this as HTMLElement)
      return i < 0 ? new DOMRect(0, 0, 240, 1000) : new DOMRect(0, i * ROW + ty(this as HTMLElement), 240, ROW)
    })
    list.setPointerCapture = () => {}
    list.releasePointerCapture = () => {}
    vi.stubGlobal('requestAnimationFrame', (cb: FrameRequestCallback) => (cb(0), 0))
    vi.stubGlobal('cancelAnimationFrame', () => {})
    const action = sortableList(list, { enabled: () => true, onDrop, onHoverSection: () => {} })
    return { list, rows, action }
  }
  const point = (type: string, y: number) =>
    new PointerEvent(type, { bubbles: true, cancelable: true, clientX: 20, clientY: y, button: 0, buttons: 1, pointerId: 1 })

  afterEach(() => {
    vi.restoreAllMocks()
    vi.unstubAllGlobals()
    document.body.innerHTML = ''
  })

  it('eases back like a cancel instead of snapping', () => {
    const onDrop = vi.fn(() => false)
    const { list, rows, action } = setup(onDrop)
    rows[0].dispatchEvent(point('pointerdown', 10))
    window.dispatchEvent(point('pointermove', 20))
    window.dispatchEvent(point('pointermove', 60))
    expect(rows[1].style.transform).toBe('translate3d(0, -34px, 0)')
    window.dispatchEvent(point('pointerup', 60))
    expect(onDrop).toHaveBeenCalledOnce()
    expect(list.classList.contains('settling')).toBe(false)
    // the others glide home (still transitioned), the lifted one returns
    expect(rows[1].style.transform).toBe('')
    expect(rows[1].classList.contains('shift')).toBe(true)
    expect(rows[0].classList.contains('returning')).toBe(true)
    action.destroy()
  })
})

describe('edgeSpeed (#193)', () => {
  // a 400px-high view of the list; the edge zone is EDGE_ZONE px at each end
  it('is still away from both edges', () => {
    expect(edgeSpeed(200, 400)).toBe(0)
    expect(edgeSpeed(EDGE_ZONE, 400)).toBe(0)
    expect(edgeSpeed(400 - EDGE_ZONE, 400)).toBe(0)
  })

  it('scrolls up near the top and down near the bottom, faster the closer it gets', () => {
    const near = edgeSpeed(EDGE_ZONE / 2, 400)
    const nearer = edgeSpeed(EDGE_ZONE / 4, 400)
    expect(near).toBeLessThan(0)
    expect(nearer).toBeLessThan(near)
    expect(edgeSpeed(400 - EDGE_ZONE / 2, 400)).toBeCloseTo(-near)
  })

  it('is proportional to how far into the zone the pointer is', () => {
    expect(edgeSpeed(400 - EDGE_ZONE / 2, 400)).toBeCloseTo(EDGE_MAX_SPEED / 2)
    expect(edgeSpeed(400, 400)).toBeCloseTo(EDGE_MAX_SPEED)
  })

  it('past the edge (outside the list) it runs at full speed, no faster', () => {
    expect(edgeSpeed(-50, 400)).toBeCloseTo(-EDGE_MAX_SPEED)
    expect(edgeSpeed(900, 400)).toBeCloseTo(EDGE_MAX_SPEED)
  })

  it('a short list keeps a middle where nothing scrolls', () => {
    expect(edgeSpeed(30, 60)).toBe(0)
  })
})

describe('easeSpeed (#193)', () => {
  it('eases toward the target speed rather than jumping to it', () => {
    const v = easeSpeed(0, 1, 16, false)
    expect(v).toBeGreaterThan(0)
    expect(v).toBeLessThan(1)
    expect(easeSpeed(v, 1, 16, false)).toBeGreaterThan(v)
  })

  it('under reduced motion it goes straight to the target: no easing', () => {
    expect(easeSpeed(0, 1, 16, true)).toBe(1)
    expect(easeSpeed(1, 0, 16, true)).toBe(0)
  })

  it('settles to a stop once the target is 0', () => {
    let v = 1
    for (let i = 0; i < 60; i++) v = easeSpeed(v, 0, 16, false)
    expect(v).toBe(0)
  })
})

describe('sortableList: edge auto-scroll (#193)', () => {
  // ten rows, 34px each, in a list whose view is 200px high and scrolls to 340 - 200 = 140
  const ROW = 34
  let frames: FrameRequestCallback[] = []
  let now = 0
  function setup() {
    const rowsHtml = Array.from({ length: 10 }, (_, i) => `<div class="row" data-channel="R${i}"></div>`).join('')
    document.body.innerHTML = `<div class="list"><div class="cat top">${rowsHtml}</div></div>`
    const list = document.querySelector<HTMLElement>('.list')!
    const rows = [...list.querySelectorAll<HTMLElement>('.row')]
    let scrollTop = 0
    Object.defineProperty(list, 'scrollTop', { get: () => scrollTop, set: (v: number) => (scrollTop = Math.max(0, Math.min(140, v))), configurable: true })
    Object.defineProperty(list, 'clientHeight', { get: () => 200, configurable: true })
    Object.defineProperty(list, 'scrollHeight', { get: () => 340, configurable: true })
    const ty = (el: HTMLElement) => Number(/, (-?[\d.]+)px/.exec(el.style.transform)?.[1] ?? 0)
    vi.spyOn(Element.prototype, 'getBoundingClientRect').mockImplementation(function (this: Element) {
      const i = rows.indexOf(this as HTMLElement)
      return i < 0 ? new DOMRect(0, 0, 240, 200) : new DOMRect(0, i * ROW - scrollTop + ty(this as HTMLElement), 240, ROW)
    })
    list.setPointerCapture = () => {}
    list.releasePointerCapture = () => {}
    frames = []
    now = 0
    vi.stubGlobal('requestAnimationFrame', (cb: FrameRequestCallback) => (frames.push(cb), frames.length))
    vi.stubGlobal('cancelAnimationFrame', (id: number) => {
      // ids are 1-based positions in the current batch
      if (frames[id - 1]) frames[id - 1] = () => {}
    })
    const onDrop = vi.fn(() => true)
    const action = sortableList(list, { enabled: () => true, onDrop, onHoverSection: () => {} })
    return { list, rows, action, onDrop }
  }
  /** Run `n` frames, 16ms apart. */
  function run(n: number) {
    for (let i = 0; i < n; i++) {
      const fs = frames
      frames = []
      now += 16
      for (const f of fs) f(now)
    }
  }
  const point = (type: string, y: number) =>
    new PointerEvent(type, { bubbles: true, cancelable: true, clientX: 20, clientY: y, button: 0, buttons: 1, pointerId: 1 })

  afterEach(() => {
    vi.restoreAllMocks()
    vi.unstubAllGlobals()
    document.body.innerHTML = ''
  })

  it('held near the bottom edge, the list scrolls down, up to its end', () => {
    const { list, rows, action } = setup()
    rows[1].dispatchEvent(point('pointerdown', 40))
    window.dispatchEvent(point('pointermove', 50))
    run(2)
    expect(list.scrollTop).toBe(0)
    window.dispatchEvent(point('pointermove', 198))
    run(10)
    expect(list.scrollTop).toBeGreaterThan(0)
    run(200)
    expect(list.scrollTop).toBe(140)
    action.destroy()
  })

  it('held near the top edge, it scrolls back up', () => {
    const { list, rows, action } = setup()
    list.scrollTop = 100
    rows[5].dispatchEvent(point('pointerdown', 5 * ROW - 100 + 10))
    window.dispatchEvent(point('pointermove', 2))
    run(200)
    expect(list.scrollTop).toBe(0)
    action.destroy()
  })

  it('the lifted row stays under the pointer while the list scrolls', () => {
    const { list, rows, action } = setup()
    rows[1].dispatchEvent(point('pointerdown', 40))
    window.dispatchEvent(point('pointermove', 195))
    run(30)
    expect(list.scrollTop).toBeGreaterThan(0)
    // grabbed 6px below its top: it stays drawn 6px above the pointer
    expect(rows[1].getBoundingClientRect().top).toBeCloseTo(195 - 6, 0)
    action.destroy()
  })

  it('stops on Escape', () => {
    const { list, rows, action } = setup()
    rows[1].dispatchEvent(point('pointerdown', 40))
    window.dispatchEvent(point('pointermove', 198))
    run(10)
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))
    const s = list.scrollTop
    expect(s).toBeGreaterThan(0)
    run(30)
    expect(list.scrollTop).toBe(s)
    action.destroy()
  })

  it('stops on drop, and the drop lands where the scrolled pointer is', () => {
    const { list, rows, action, onDrop } = setup()
    rows[0].dispatchEvent(point('pointerdown', 10))
    window.dispatchEvent(point('pointermove', 198))
    run(300)
    expect(list.scrollTop).toBe(140)
    window.dispatchEvent(point('pointerup', 198))
    // the pointer at 198 + 140 = 338: past every other row, the last slot
    expect(onDrop).toHaveBeenCalledWith({ kind: 'channel', id: 'R0', to: { category: null, index: 9 } })
    const s = list.scrollTop
    run(30)
    expect(list.scrollTop).toBe(s)
    action.destroy()
  })

  it('under reduced motion it still scrolls, at its full speed from the first frame (no ramp)', () => {
    motion.reducedMotion = true
    try {
      const { list, rows, action } = setup()
      rows[1].dispatchEvent(point('pointerdown', 40))
      window.dispatchEvent(point('pointermove', 200))
      run(1)
      expect(list.scrollTop).toBeCloseTo(EDGE_MAX_SPEED * 16)
      action.destroy()
    } finally {
      motion.reducedMotion = false
    }
  })

  it('moving out of the edge zone stops it', () => {
    const { list, rows, action } = setup()
    rows[1].dispatchEvent(point('pointerdown', 40))
    window.dispatchEvent(point('pointermove', 198))
    run(4)
    window.dispatchEvent(point('pointermove', 100))
    run(60)
    const s = list.scrollTop
    run(30)
    expect(list.scrollTop).toBe(s)
    expect(s).toBeLessThan(140)
    action.destroy()
  })
})
