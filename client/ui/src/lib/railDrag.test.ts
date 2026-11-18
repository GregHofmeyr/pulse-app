// @vitest-environment jsdom
// Drag to reorder the server tabs (#185).
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { railDrag } from './railDrag'

// jsdom has no PointerEvent: a MouseEvent with a pointerId stands in
function pointer(type: string, el: Element, x: number, id = 1) {
  const e = new MouseEvent(type, { bubbles: true, cancelable: true, clientX: x, button: 0 })
  Object.defineProperty(e, 'pointerId', { value: id })
  el.dispatchEvent(e)
}

let nav: HTMLElement
let action: ReturnType<typeof railDrag>
const onDrop = vi.fn()
// three 100px tabs, 6px apart: A at 0, B at 106, C at 212
beforeEach(() => {
  onDrop.mockReset()
  nav = document.createElement('nav')
  ;['A', 'B', 'C'].forEach((id, i) => {
    const b = document.createElement('button')
    b.dataset.server = id
    b.textContent = id
    b.getBoundingClientRect = () => ({ left: i * 106, right: i * 106 + 100, width: 100, top: 0, bottom: 36, height: 36, x: i * 106, y: 0, toJSON: () => ({}) })
    nav.append(b)
  })
  document.body.append(nav)
  action = railDrag(nav, { onDrop })
})
afterEach(() => {
  action.destroy()
  document.body.innerHTML = ''
})
const tab = (id: string) => nav.querySelector<HTMLElement>(`[data-server="${id}"]`)!

describe('server rail drag', () => {
  it('dragging A past C drops it at the end; the others make room as it goes', () => {
    pointer('pointerdown', tab('A'), 50)
    pointer('pointermove', nav, 160)
    expect(tab('A').style.transform).toBe('translateX(110px)')
    expect(tab('B').style.transform).toBe('translateX(-106px)')
    expect(tab('C').style.transform).toBe('')
    pointer('pointermove', nav, 290)
    pointer('pointerup', nav, 290)
    expect(onDrop).toHaveBeenCalledWith('A', 2)
    expect([tab('A'), tab('B'), tab('C')].map((t) => t.style.transform)).toEqual(['', '', ''])
  })

  it('dragging C to the front', () => {
    pointer('pointerdown', tab('C'), 260)
    pointer('pointermove', nav, 20)
    pointer('pointerup', nav, 20)
    expect(onDrop).toHaveBeenCalledWith('C', 0)
  })

  it('a click (under the threshold) is a click: no drop, and the click goes through', () => {
    const clicked = vi.fn()
    tab('B').addEventListener('click', clicked)
    pointer('pointerdown', tab('B'), 150)
    pointer('pointermove', nav, 152)
    pointer('pointerup', nav, 152)
    tab('B').click()
    expect(onDrop).not.toHaveBeenCalled()
    expect(clicked).toHaveBeenCalledOnce()
  })

  it('the click a drag ends with is swallowed', () => {
    const clicked = vi.fn()
    tab('A').addEventListener('click', clicked)
    pointer('pointerdown', tab('A'), 50)
    pointer('pointermove', nav, 290)
    pointer('pointerup', nav, 290)
    tab('A').click()
    expect(clicked).not.toHaveBeenCalled()
  })

  it('a drop where it started, and Escape, change nothing', () => {
    pointer('pointerdown', tab('B'), 150)
    pointer('pointermove', nav, 170)
    pointer('pointerup', nav, 170)
    expect(onDrop).not.toHaveBeenCalled()
    pointer('pointerdown', tab('B'), 150)
    pointer('pointermove', nav, 300)
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }))
    expect(tab('B').style.transform).toBe('')
    pointer('pointerup', nav, 300)
    expect(onDrop).not.toHaveBeenCalled()
  })

  it('off while disabled', () => {
    action.update({ onDrop, disabled: true })
    pointer('pointerdown', tab('A'), 50)
    pointer('pointermove', nav, 290)
    pointer('pointerup', nav, 290)
    expect(onDrop).not.toHaveBeenCalled()
  })
})

describe('another kind of item (#467: the profile editor drags [data-badge])', () => {
  it('drags the items its `item` attribute names', () => {
    const row = document.createElement('ol')
    ;['x', 'y'].forEach((id, i) => {
      const b = document.createElement('button')
      b.dataset.badge = id
      b.getBoundingClientRect = () => ({ left: i * 60, right: i * 60 + 52, width: 52, top: 0, bottom: 52, height: 52, x: i * 60, y: 0, toJSON: () => ({}) })
      row.append(b)
    })
    document.body.append(row)
    const drop = vi.fn()
    const a = railDrag(row, { onDrop: drop, item: 'badge' })
    pointer('pointerdown', row.querySelector('[data-badge="x"]')!, 20)
    pointer('pointermove', row, 110)
    pointer('pointerup', row, 110)
    expect(drop).toHaveBeenCalledWith('x', 1)
    a.destroy()
  })
})
