// @vitest-environment jsdom
// Staying at the bottom (#292): a message that grows after it's drawn (a link's card unfurling later, an image loading,
// reactions) keeps a list that was at the bottom at the bottom; a list scrolled up, or moved by a jump, stays put.
// jsdom has no layout: each message is 100px tall unless the test grows it, and ResizeObserver is a stand-in fired by
// hand (after any scroll event, the order a frame runs them in).
import { flushSync, mount, tick, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Message } from '../lib/protocol/Message'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => []) }))

import TextChannel from './TextChannel.svelte'
import { app, resetState, ui } from '../lib/store.svelte'
import { applyReady, emptyState, loadLatest } from '../lib/state'
import { testReady } from '../lib/testReady'

const msg = (n: number): Message => ({
  id: `M${String(n).padStart(4, '0')}`, channel_id: 'C1', author_id: 'U2', kind: 'normal', content: `m${n}`,
  reply_to_id: null, created_at: '2026-10-01T00:00:00.000Z', edited_at: null, deleted: false, mentions: [], reactions: [], thread_root_id: null, quote: null, qoty_card: null, clip: null, embeds: [],
})
const ROW = 100
const LIST = 1000
const grown = new Map<string, number>()
const tops = new WeakMap<Element, number>()
const isList = (el: Element) => el.classList.contains('list')
const rows = (l: Element) => [...l.querySelectorAll<HTMLElement>('[data-message-id]')]
const rowHeight = (el: HTMLElement) => ROW + (grown.get(el.dataset.messageId!) ?? 0)
const list = () => document.querySelector<HTMLElement>('.list')!

// the stand-in ResizeObserver: what's observed, and a way to run the callbacks as a frame would
const observers = new Set<{ cb: ResizeObserverCallback; els: Set<Element>; self: ResizeObserver }>()
let lastTop = -1
function frame() {
  // a scroll event first, as in a real frame, and only when the view moved (content growing under it doesn't fire one)
  if (list().scrollTop !== lastTop) list().dispatchEvent(new Event('scroll'))
  lastTop = list().scrollTop
  for (const o of [...observers]) if (o.els.size) o.cb([], o.self)
}

let cmp: ReturnType<typeof mount> | undefined
const saved: [object, string, PropertyDescriptor | undefined][] = []
function fake(proto: object, key: string, desc: PropertyDescriptor) {
  saved.push([proto, key, Object.getOwnPropertyDescriptor(proto, key)])
  Object.defineProperty(proto, key, { configurable: true, ...desc })
}

describe('TextChannel: stays at the bottom as messages grow', () => {
  beforeEach(() => {
    observers.clear()
    lastTop = -1
    vi.stubGlobal('ResizeObserver', class {
      o: { cb: ResizeObserverCallback; els: Set<Element>; self: ResizeObserver }
      constructor(cb: ResizeObserverCallback) {
        this.o = { cb, els: new Set(), self: this as unknown as ResizeObserver }
        observers.add(this.o)
      }
      observe(el: Element) { this.o.els.add(el) }
      unobserve(el: Element) { this.o.els.delete(el) }
      disconnect() { this.o.els.clear(); observers.delete(this.o) }
    })
    fake(Element.prototype, 'scrollHeight', { get(this: Element) { return isList(this) ? rows(this).reduce((s, r) => s + rowHeight(r), 0) : 0 } })
    fake(Element.prototype, 'clientHeight', { get(this: Element) { return isList(this) ? LIST : 0 } })
    fake(Element.prototype, 'scrollTop', {
      get(this: Element) { return tops.get(this) ?? 0 },
      set(this: Element, v: number) { tops.set(this, Math.max(0, Math.min(v, this.scrollHeight - this.clientHeight))) },
    })
    fake(Element.prototype, 'getBoundingClientRect', { value(this: Element) {
      const l = this.closest('.list')
      let top = 0
      let h = 0
      if (l && this !== l) {
        const rs = rows(l)
        const i = rs.indexOf(this as HTMLElement)
        if (i >= 0) {
          top = rs.slice(0, i).reduce((s, r) => s + rowHeight(r), 0) - l.scrollTop
          h = rowHeight(this as HTMLElement)
        }
      }
      return { top, bottom: top + h, left: 0, right: 0, width: 0, height: h, x: 0, y: top, toJSON() {} } as DOMRect
    } })
    Element.prototype.scrollTo = function (this: Element, o?: ScrollToOptions | number) { if (typeof o === 'object' && o.top !== undefined) this.scrollTop = o.top }
    // a jump puts its message in the middle
    Element.prototype.scrollIntoView = function (this: Element) {
      const l = this.closest('.list')
      if (l) l.scrollTop += this.getBoundingClientRect().top - (LIST - rowHeight(this as HTMLElement)) / 2
    }
    resetState()
    ui.jump = null
    grown.clear()
    app.state = loadLatest(applyReady(emptyState(), testReady([{ id: 'C1', server_id: 'S1', kind: 'text', name: 'general', position: 0 }])), 'C1', Array.from({ length: 20 }, (_, i) => msg(i + 1)))
  })
  afterEach(() => {
    if (cmp) unmount(cmp)
    cmp = undefined
    document.body.innerHTML = ''
    for (const [proto, key, desc] of saved.reverse()) desc ? Object.defineProperty(proto, key, desc) : delete (proto as Record<string, unknown>)[key]
    saved.length = 0
    vi.unstubAllGlobals()
  })
  const show = async () => {
    cmp = mount(TextChannel, { target: document.body, props: { channelId: 'C1', serverId: 'S1' } })
    flushSync()
    await tick()
    flushSync()
    frame()
  }
  const bottom = () => list().scrollHeight - list().clientHeight

  it('at the bottom: a message that grows later (its link card unfurling) keeps the list at the bottom', async () => {
    await show()
    expect(list().scrollTop).toBe(bottom())
    grown.set('M0020', 240) // the newest message's card arrives
    frame()
    expect(list().scrollTop).toBe(bottom())
    grown.set('M0019', 60) // one above it gets a reaction row
    frame()
    expect(list().scrollTop).toBe(bottom())
  })

  it('scrolled up: a message growing leaves the view where it is', async () => {
    await show()
    list().scrollTop -= 500
    frame()
    const at = list().scrollTop
    grown.set('M0020', 240)
    frame()
    expect(list().scrollTop).toBe(at)
  })

  it('a little above the bottom (within the threshold) still counts as at the bottom', async () => {
    await show()
    list().scrollTop -= 20
    frame()
    grown.set('M0020', 240)
    frame()
    expect(list().scrollTop).toBe(bottom())
  })

  it('a jump that moved the view away is not pulled back down when its messages grow', async () => {
    await show()
    ui.jump = { channelId: 'C1', messageId: 'M0005', seq: 1 }
    for (let i = 0; i < 3; i++) await tick()
    flushSync()
    expect(document.querySelector('[data-message-id="M0005"]')!.classList.contains('jump-flash')).toBe(true)
    const at = list().scrollTop
    expect(at).toBeLessThan(bottom())
    grown.set('M0005', 200) // e.g. its image loads in the same frame as the jump
    grown.set('M0020', 200)
    frame()
    expect(list().scrollTop).toBe(at)
  })

  it("a detached window (a jump into the past) isn't pulled down: its bottom isn't the latest", async () => {
    app.state = { ...app.state, history: { ...app.state.history, C1: { ...app.state.history.C1!, end: false } } }
    await show()
    list().scrollTop = bottom()
    frame()
    const at = list().scrollTop
    grown.set('M0020', 240)
    frame()
    expect(list().scrollTop).toBe(at)
  })

  it('back at the bottom after scrolling up: pinned again', async () => {
    await show()
    list().scrollTop -= 500
    frame()
    list().scrollTop = bottom()
    frame()
    grown.set('M0020', 240)
    frame()
    expect(list().scrollTop).toBe(bottom())
  })
})
