// @vitest-environment jsdom
// Drag to reorder the channel list (#55, #54): pointer events on rows and category headings, with a stubbed layout
// (jsdom lays nothing out): every row 34px, every heading 30px, every voice occupant 30px, stacked without gaps from
// the list's top at 0.
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const invoke = vi.fn()
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...a: unknown[]) => invoke(...a) }))
vi.mock('@tauri-apps/api/event', () => ({ listen: async () => () => {} }))

import ChannelList from './ChannelList.svelte'
import { app } from '../lib/store.svelte'
import { applyEvent, applyReady, emptyState } from '../lib/state'
import { drafts, resetSaves } from '../lib/layoutDraft.svelte'
import { isCollapsed, resetCollapsed, setCollapsed } from '../lib/sidebarSections.svelte'
import { testReady } from '../lib/testReady'
import type { Category } from '../lib/protocol/Category'
import type { Channel } from '../lib/protocol/Channel'

const cat = (id: string, name: string, position: number): Category => ({ id, server_id: 'S1', name, position })
const CATS: Category[] = [cat('C1', 'Text Channels', 0), cat('C2', 'Gaming', 1), cat('C3', 'Voice Channels', 2)]
const T0: Channel = { id: 'T0', server_id: 'S1', kind: 'text', name: 'welcome', position: 0 }
const REST: Channel[] = [
  { id: 'T1', server_id: 'S1', kind: 'text', name: 'general', position: 0, category_id: 'C1' },
  { id: 'T2', server_id: 'S1', kind: 'text', name: 'memes', position: 1, category_id: 'C1' },
  { id: 'Q1', server_id: 'S1', kind: 'quotes', name: 'quotes', position: 0, category_id: 'C2' },
  { id: 'V2', server_id: 'S1', kind: 'voice', name: 'Games', position: 1, category_id: 'C2' },
  { id: 'V1', server_id: 'S1', kind: 'voice', name: 'Lounge', position: 0, category_id: 'C3' },
]

// ---- a layout for jsdom ----
const HEIGHT = (el: Element) => (el.matches('.row') ? 34 : el.matches('.label') ? 30 : el.matches('.occupant') ? 30 : 20)
const translateY = (el: Element) => Number(/translate3d\(0(?:px)?, (-?[\d.]+)px/.exec((el as HTMLElement).style?.transform ?? '')?.[1] ?? 0)
/** Each element's untransformed box: the list's children (and a section's children) stacked in order. */
function layoutTable(): Map<Element, { top: number; height: number }> {
  const out = new Map<Element, { top: number; height: number }>()
  const list = document.querySelector('.list')
  if (!list) return out
  out.set(list, { top: 0, height: 1000 })
  let y = 0
  for (const child of list.children) {
    if (child.classList.contains('drop-line')) continue
    if (child.matches('.cat')) {
      const top = y
      for (const k of child.children) {
        out.set(k, { top: y, height: HEIGHT(k) })
        y += HEIGHT(k)
      }
      out.set(child, { top, height: y - top })
    } else {
      out.set(child, { top: y, height: HEIGHT(child) })
      y += HEIGHT(child)
    }
  }
  return out
}
function rect(this: Element): DOMRect {
  const box = layoutTable().get(this)
  if (!box) return new DOMRect(0, 0, 0, 0)
  // a transform moves the box, as in a browser (this element's and a moved section's)
  let dy = 0
  for (let el: Element | null = this; el && !el.matches('.list'); el = el.parentElement) dy += translateY(el)
  return new DOMRect(0, box.top + dy, 240, box.height)
}

// ---- pointer input ----
const point = (type: string, y: number, init: PointerEventInit = {}) =>
  new PointerEvent(type, { bubbles: true, cancelable: true, composed: true, clientX: 20, clientY: y, button: 0, buttons: 1, pointerId: 1, ...init })
const down = (el: Element, y: number, init: PointerEventInit = {}) => void el.dispatchEvent(point('pointerdown', y, init))
const move = (y: number) => {
  window.dispatchEvent(point('pointermove', y))
  flushSync()
}
const up = (y: number) => {
  window.dispatchEvent(point('pointerup', y, { buttons: 0 }))
  flushSync()
}
/** A whole drag: press at `from`, past the threshold, to `to`, let go, and the click the release makes (with the
 *  pointer captured by the list, a browser clicks the list; the pressed row may be gone by then). */
function drag(el: HTMLElement, from: number, to: number) {
  down(el, from)
  move(from + (to > from ? 5 : -5))
  move(to)
  up(to)
  const target = el.isConnected ? el : document.querySelector<HTMLElement>('.list')!
  target.click()
  flushSync()
}

const rowOf = (id: string) => document.querySelector<HTMLElement>(`.row[data-channel="${id}"]`)!
const headingOf = (id: string) => document.querySelector<HTMLElement>(`.cat[data-category="${id}"] > .label button.section`)!
const savedLayout = () => invoke.mock.calls.filter(([c]) => c === 'save_layout').map(([, a]) => (a as { layout: unknown }).layout)
const transformed = () => [...document.querySelectorAll<HTMLElement>('.list *')].filter((e) => e.style.transform)

let cmp: ReturnType<typeof mount> | undefined
let onSelect = vi.fn()
function show(over: { top?: boolean; cats?: Category[]; member?: boolean } = {}) {
  const channels = [...(over.top === false ? [] : [T0]), ...REST]
  const layouts = [{ server_id: 'S1', revision: 3, categories: over.cats ?? CATS, channels: [] }]
  app.state = applyReady(emptyState(), testReady(channels, { layouts, ...(over.member === false ? { members: [] } : {}) }))
  onSelect = vi.fn()
  cmp = mount(ChannelList, {
    target: document.body,
    props: { serverId: 'S1', activeChannelId: null, voiceChannelId: null, speaking: new Set<string>(), onSelect, onJoinVoice: () => {}, onMessage: () => {}, onVoiceSettings: () => {} },
  })
  flushSync()
}

beforeEach(() => {
  localStorage.clear()
  resetCollapsed()
  resetSaves()
  invoke.mockReset()
  // a save stays in flight: the draft keeps showing
  invoke.mockImplementation((c: string) => (c === 'save_layout' ? new Promise(() => {}) : Promise.resolve(undefined)))
  vi.spyOn(Element.prototype, 'getBoundingClientRect').mockImplementation(rect)
  if (!Element.prototype.setPointerCapture) Element.prototype.setPointerCapture = () => {}
  if (!Element.prototype.releasePointerCapture) Element.prototype.releasePointerCapture = () => {}
  // one frame per request, at once
  vi.stubGlobal('requestAnimationFrame', (cb: FrameRequestCallback) => {
    cb(0)
    return 0
  })
  vi.stubGlobal('cancelAnimationFrame', () => {})
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
  resetSaves()
  vi.useRealTimers()
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

// The layout, top to bottom (y):
//   T0 welcome 0–34
//   C1 heading 34–64, T1 general 64–98, T2 memes 98–132
//   C2 heading 132–162, Q1 quotes 162–196, V2 Games 196–230
//   C3 heading 230–260, V1 Lounge 260–294
describe('dragging a channel', () => {
  it('within its category: the rows make room while it moves, and the drop saves the new order', () => {
    show()
    down(rowOf('T1'), 80)
    move(85)
    move(125)
    // lifted, following the pointer; memes moved up into its place; a drop line shows
    expect(rowOf('T1').classList.contains('lifted')).toBe(true)
    expect(rowOf('T1').style.transform).toBe('translate3d(0, 45px, 0)')
    expect(rowOf('T2').style.transform).toBe('translate3d(0, -34px, 0)')
    expect(document.querySelector('.list')!.classList.contains('dragging')).toBe(true)
    expect(document.querySelector('.drop-line')).not.toBeNull()
    up(125)
    rowOf('T1').click()
    expect(savedLayout()).toEqual([{ revision: 3, uncategorised: ['T0'], categories: [{ id: 'C1', channels: ['T2', 'T1'] }, { id: 'C2', channels: ['Q1', 'V2'] }, { id: 'C3', channels: ['V1'] }] }])
    // the click that ended the drag doesn't open the channel
    expect(onSelect).not.toHaveBeenCalled()
  })

  it('the drop renders the new order with nothing left moved, lifted or dragging', () => {
    show()
    drag(rowOf('T1'), 80, 125)
    const names = [...document.querySelectorAll('.cat[data-category="C1"] .row .grow')].map((e) => e.textContent?.trim())
    expect(names).toEqual(['memes', 'general'])
    expect(transformed()).toEqual([])
    const list = document.querySelector('.list')!
    expect(list.classList.contains('dragging')).toBe(false)
    expect(list.classList.contains('settling')).toBe(false)
    expect(document.querySelector('.drop-line')).toBeNull()
  })

  it('into another category, between its channels', () => {
    show()
    drag(rowOf('T1'), 80, 200)
    expect(savedLayout()).toEqual([{ revision: 3, uncategorised: ['T0'], categories: [{ id: 'C1', channels: ['T2'] }, { id: 'C2', channels: ['Q1', 'T1', 'V2'] }, { id: 'C3', channels: ['V1'] }] }])
  })

  it('into the uncategorised area on top', () => {
    show()
    drag(rowOf('T1'), 80, 40)
    expect(savedLayout()).toEqual([{ revision: 3, uncategorised: ['T0', 'T1'], categories: [{ id: 'C1', channels: ['T2'] }, { id: 'C2', channels: ['Q1', 'V2'] }, { id: 'C3', channels: ['V1'] }] }])
  })

  it('into the empty top area, above the first category', () => {
    // no uncategorised channels: C1 heading 0–30, T1 30–64, T2 64–98, C2 heading 98–128, …
    show({ top: false })
    drag(rowOf('T2'), 80, 4)
    expect(savedLayout()).toEqual([{ revision: 3, uncategorised: ['T2'], categories: [{ id: 'C1', channels: ['T1'] }, { id: 'C2', channels: ['Q1', 'V2'] }, { id: 'C3', channels: ['V1'] }] }])
  })

  it('into an empty category', () => {
    // C4 heading 294–324, nothing under it
    show({ cats: [...CATS, cat('C4', 'Empty', 3)] })
    drag(rowOf('T1'), 80, 318)
    expect(savedLayout()).toEqual([{ revision: 3, uncategorised: ['T0'], categories: [{ id: 'C1', channels: ['T2'] }, { id: 'C2', channels: ['Q1', 'V2'] }, { id: 'C3', channels: ['V1'] }, { id: 'C4', channels: ['T1'] }] }])
  })

  it('a voice channel carries the people in it', () => {
    show()
    app.state = applyEvent(app.state, { t: 'VoiceJoined', d: { channel_id: 'V2', user_id: 'U1', flags: { muted: false, deafened: false } } } as never, Date.now())
    flushSync()
    const occupant = document.querySelector<HTMLElement>('.occupant[data-user="U1"]')!
    // V2 196–230 with its occupant 230–260
    down(rowOf('V2'), 210)
    move(215)
    move(260)
    expect(occupant.style.transform).toBe(rowOf('V2').style.transform)
    expect(occupant.classList.contains('lifted')).toBe(true)
    up(260)
  })

  it('dropped where it started: nothing is saved', () => {
    show()
    drag(rowOf('T1'), 80, 90)
    expect(savedLayout()).toEqual([])
    expect(onSelect).not.toHaveBeenCalled()
  })

  it('under 4px of movement it is a click: the channel opens, nothing is saved', () => {
    show()
    down(rowOf('T1'), 80)
    move(83)
    up(83)
    rowOf('T1').click()
    expect(onSelect).toHaveBeenCalledWith('T1')
    expect(savedLayout()).toEqual([])
    expect(rowOf('T1').classList.contains('lifted')).toBe(false)
  })

  it('a later plain click still opens the channel, and right-click still opens its menu', () => {
    show()
    drag(rowOf('T1'), 80, 200)
    rowOf('T2').click()
    expect(onSelect).toHaveBeenCalledWith('T2')
    rowOf('T2').dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: 40, clientY: 50 }))
    flushSync()
    expect(document.querySelector('[role=menu]')).not.toBeNull()
  })

  it('only the main button drags', () => {
    show()
    down(rowOf('T1'), 80, { button: 2, buttons: 2 })
    move(200)
    up(200)
    expect(rowOf('T1').classList.contains('lifted')).toBe(false)
    expect(savedLayout()).toEqual([])
  })

  it('Escape cancels: everything eases back and nothing is saved', () => {
    show()
    down(rowOf('T1'), 80)
    move(85)
    move(200)
    const esc = new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true })
    window.dispatchEvent(esc)
    flushSync()
    expect(esc.defaultPrevented).toBe(true)
    expect(transformed()).toEqual([])
    expect(document.querySelector('.drop-line')).toBeNull()
    expect(document.querySelector('.list')!.classList.contains('dragging')).toBe(false)
    up(200)
    rowOf('T1').click()
    expect(savedLayout()).toEqual([])
    expect(onSelect).not.toHaveBeenCalled()
  })

  it('cancelled where it started: nothing has to ease back, so nothing is left lifted or waiting', () => {
    show()
    down(rowOf('T1'), 80)
    move(85)
    move(80)
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }))
    flushSync()
    expect(document.querySelectorAll('.list .shift, .list .lifted, .list .returning').length).toBe(0)
    up(80)
  })

  it("someone else's change mid-drag cancels it: everything eases back and nothing is saved", () => {
    show()
    down(rowOf('T1'), 80)
    move(85)
    move(125)
    // a friend moved memes to the top meanwhile (revision 4)
    const layout = {
      server_id: 'S1', revision: 4, categories: CATS,
      channels: [
        { channel_id: 'T0', category_id: null, position: 0 }, { channel_id: 'T2', category_id: null, position: 1 },
        { channel_id: 'T1', category_id: 'C1', position: 0 }, { channel_id: 'Q1', category_id: 'C2', position: 0 },
        { channel_id: 'V2', category_id: 'C2', position: 1 }, { channel_id: 'V1', category_id: 'C3', position: 0 },
      ],
    }
    app.state = applyEvent(app.state, { t: 'LayoutChanged', d: { layout } } as never, Date.now())
    flushSync()
    expect(transformed()).toEqual([])
    expect(document.querySelector('.drop-line')).toBeNull()
    expect(document.querySelector('.list')!.classList.contains('dragging')).toBe(false)
    expect(rowOf('T1').classList.contains('returning')).toBe(true)
    move(200)
    up(200)
    document.querySelector<HTMLElement>('.list')!.click()
    expect(savedLayout()).toEqual([])
    expect(onSelect).not.toHaveBeenCalled()
  })

  it('your own save landing mid-drag carries on, and the drop goes on the revision it made', async () => {
    let land: ((l: unknown) => void) | undefined
    invoke.mockImplementation((c: string) =>
      c !== 'save_layout' ? Promise.resolve(undefined) : land ? new Promise(() => {}) : new Promise((r) => (land = r)))
    show()
    drag(rowOf('T1'), 80, 125)
    // memes 64–98, general 98–132: memes back below general
    down(rowOf('T2'), 80)
    move(85)
    land!({
      server_id: 'S1', revision: 4, categories: CATS,
      channels: [
        { channel_id: 'T0', category_id: null, position: 0 }, { channel_id: 'T2', category_id: 'C1', position: 0 },
        { channel_id: 'T1', category_id: 'C1', position: 1 }, { channel_id: 'Q1', category_id: 'C2', position: 0 },
        { channel_id: 'V2', category_id: 'C2', position: 1 }, { channel_id: 'V1', category_id: 'C3', position: 0 },
      ],
    })
    for (let i = 0; i < 4; i++) await Promise.resolve()
    flushSync()
    expect(rowOf('T2').classList.contains('lifted')).toBe(true)
    move(125)
    up(125)
    expect(savedLayout()[1]).toEqual({ revision: 4, uncategorised: ['T0'], categories: [{ id: 'C1', channels: ['T1', 'T2'] }, { id: 'C2', channels: ['Q1', 'V2'] }, { id: 'C3', channels: ['V1'] }] })
  })

  it('a cancelled pointer cancels the drag', () => {
    show()
    down(rowOf('T1'), 80)
    move(85)
    move(200)
    window.dispatchEvent(point('pointercancel', 200))
    flushSync()
    expect(transformed()).toEqual([])
    expect(savedLayout()).toEqual([])
  })

  it('hovering a collapsed category for a moment opens it, and the drop lands where the pointer is', () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] })
    setCollapsed('cat:C2', true)
    show()
    // collapsed C2: heading 132–162, then C3 heading 162–192
    expect(document.querySelector('.row[data-channel="Q1"]')).toBeNull()
    down(rowOf('T1'), 80)
    move(85)
    move(155)
    vi.advanceTimersByTime(599)
    flushSync()
    expect(isCollapsed('cat:C2')).toBe(true)
    vi.advanceTimersByTime(1)
    flushSync()
    expect(isCollapsed('cat:C2')).toBe(false)
    expect(headingOf('C2').getAttribute('aria-expanded')).toBe('true')
    // the rows that just appeared ease like the rest, rather than jumping
    expect(rowOf('Q1').classList.contains('shift')).toBe(true)
    expect(rowOf('V2').classList.contains('shift')).toBe(true)
    // opened (re-measured): Q1 162–196, V2 196–230; the pointer is now over Q1's top half
    up(170)
    expect(savedLayout()).toEqual([{ revision: 3, uncategorised: ['T0'], categories: [{ id: 'C1', channels: ['T2'] }, { id: 'C2', channels: ['T1', 'Q1', 'V2'] }, { id: 'C3', channels: ['V1'] }] }])
  })

  it('dropped onto a collapsed category at once: it lands first there, and the category opens', () => {
    setCollapsed('cat:C2', true)
    show()
    drag(rowOf('T1'), 80, 155)
    expect(savedLayout()).toEqual([{ revision: 3, uncategorised: ['T0'], categories: [{ id: 'C1', channels: ['T2'] }, { id: 'C2', channels: ['T1', 'Q1', 'V2'] }, { id: 'C3', channels: ['V1'] }] }])
    expect(isCollapsed('cat:C2')).toBe(false)
  })
})

describe('dragging a category', () => {
  it('moves it, with its channels, above another', () => {
    show()
    // C1 section 34–132, C2 132–230, C3 230–294; C3's heading up into C2's top half
    const heading = headingOf('C3')
    down(heading, 245)
    move(240)
    move(150)
    const c3 = document.querySelector<HTMLElement>('.cat[data-category="C3"]')!
    expect(c3.classList.contains('lifted')).toBe(true)
    expect(document.querySelector<HTMLElement>('.cat[data-category="C2"]')!.style.transform).toBe('translate3d(0, 64px, 0)')
    up(150)
    heading.click()
    expect(savedLayout()).toEqual([{ revision: 3, uncategorised: ['T0'], categories: [{ id: 'C1', channels: ['T1', 'T2'] }, { id: 'C3', channels: ['V1'] }, { id: 'C2', channels: ['Q1', 'V2'] }] }])
    // the click that ended the drag didn't collapse it
    expect(isCollapsed('cat:C3')).toBe(false)
  })

  it('never above the uncategorised area', () => {
    show()
    drag(headingOf('C2'), 145, 2)
    expect(savedLayout()).toEqual([{ revision: 3, uncategorised: ['T0'], categories: [{ id: 'C2', channels: ['Q1', 'V2'] }, { id: 'C1', channels: ['T1', 'T2'] }, { id: 'C3', channels: ['V1'] }] }])
  })

  it("the heading's + doesn't start a drag", () => {
    show()
    const add = document.querySelector<HTMLElement>('.cat[data-category="C3"] > .label .add')!
    down(add, 245)
    move(150)
    up(150)
    expect(savedLayout()).toEqual([])
    expect(document.querySelector('.lifted')).toBeNull()
  })
})

describe('who may drag', () => {
  it("someone who isn't a member can't drag", () => {
    show({ member: false })
    drag(rowOf('T1'), 80, 200)
    drag(headingOf('C3'), 245, 150)
    expect(document.querySelector('.lifted')).toBeNull()
    expect(savedLayout()).toEqual([])
    // their click still opens the channel
    rowOf('T1').click()
    expect(onSelect).toHaveBeenCalledWith('T1')
  })

  it("an older server's kind sections don't drag", () => {
    app.state = applyReady(emptyState(), testReady([{ id: 'T1', server_id: 'S1', kind: 'text', name: 'general', position: 0 }, { id: 'T2', server_id: 'S1', kind: 'text', name: 'memes', position: 1 }]))
    cmp = mount(ChannelList, {
      target: document.body,
      props: { serverId: 'S1', activeChannelId: null, voiceChannelId: null, speaking: new Set<string>(), onSelect, onJoinVoice: () => {}, onMessage: () => {}, onVoiceSettings: () => {} },
    })
    flushSync()
    drag(rowOf('T1'), 50, 120)
    expect(document.querySelector('.lifted')).toBeNull()
    expect(savedLayout()).toEqual([])
  })
})
