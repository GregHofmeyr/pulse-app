// @vitest-environment jsdom
// Drag to reorder (#55): the slot and shift maths the sortable list runs every frame, and how a drop that changes
// nothing ends (the whole drag is covered through ChannelList in ChannelListDrag.test.ts).
import { afterEach, describe, expect, it, vi } from 'vitest'
import { shiftsFor, slotFor, sortableList } from './drag'

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
