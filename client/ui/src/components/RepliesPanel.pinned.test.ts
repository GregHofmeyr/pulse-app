// @vitest-environment jsdom
// Staying at the bottom (#296, as the main list does since #292): a reply that grows after it's drawn (a link's card
// unfurling later, an image loading, reactions) keeps a panel that was at the bottom at the bottom; one scrolled up
// stays put. jsdom has no layout: each reply is 100px tall unless the test grows it, and ResizeObserver is a stand-in
// fired by hand (after any scroll event, the order a frame runs them in).
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Message } from '../lib/protocol/Message'
import { FakeIntersectionObserver, fakeMatchMedia } from '../lib/testMotion'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))

import RepliesPanel from './RepliesPanel.svelte'
import { app, handleReady, resetState } from '../lib/store.svelte'
import { addHistory, markHistory } from '../lib/state'
import { testReady } from '../lib/testReady'

const base = (id: string): Message => ({
  id, channel_id: 'C1', author_id: 'U2', kind: 'normal', content: '', reply_to_id: null, created_at: '2026-10-01T10:00:00.000Z',
  edited_at: null, deleted: false, mentions: [], reactions: [], thread_root_id: null, quote: null, qoty_card: null, clip: null, embeds: [],
})
const root: Message = {
  ...base('Q1'), content: '“hi” — sam',
  quote: {
    lines: [{ id: 'L1', speaker_id: 'U2', speaker_name: null, text: 'hi', media: null, caption: null, awards: 0, awarders: [], awarded: false }],
    year: 2026, said_on: null, reply_count: 20, last_reply_id: 'R0020', replies_seen: null, wins: [],
  },
}
const reply = (n: number): Message => ({ ...base(`R${String(n).padStart(4, '0')}`), content: `reply ${n}`, thread_root_id: 'Q1' })

const ROW = 100
const LIST = 1000
const grown = new Map<string, number>()
const tops = new WeakMap<Element, number>()
const isList = (el: Element) => el.classList.contains('list')
const rows = (l: Element) => [...l.querySelectorAll<HTMLElement>('[data-message-id]')]
const rowHeight = (el: HTMLElement) => ROW + (grown.get(el.dataset.messageId!) ?? 0)
const list = () => document.querySelector<HTMLElement>('.replies-panel .list')!

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
const settle = () => new Promise((r) => setTimeout(r, 0))

describe('RepliesPanel: stays at the bottom as replies grow', () => {
  beforeEach(() => {
    observers.clear()
    lastTop = -1
    vi.stubGlobal('IntersectionObserver', FakeIntersectionObserver)
    vi.stubGlobal('matchMedia', fakeMatchMedia(false).matchMedia)
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
    fake(Element.prototype, 'scrollTo', { value(this: Element, o?: ScrollToOptions | number) { if (typeof o === 'object' && o.top !== undefined) this.scrollTop = o.top } })
    localStorage.clear()
    resetState()
    handleReady(testReady([{ id: 'C1', server_id: 'S1', kind: 'quotes', name: 'quotes', position: 0 }]))
    grown.clear()
    invoke.mockReset()
    // the newest page, newest first as the server sends it; nothing older
    invoke.mockImplementation(async (cmd: string, args?: unknown) => {
      if (cmd !== 'list_replies') return undefined
      return (args as { before: string | null }).before ? [] : Array.from({ length: 20 }, (_, i) => reply(20 - i))
    })
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
    app.state = markHistory(addHistory({ ...app.state }, 'C1', [root]), 'C1', true)
    cmp = mount(RepliesPanel, { target: document.body, props: { root, serverId: 'S1', onClose: () => {} } })
    flushSync()
    await settle()
    flushSync()
    await settle()
    frame()
  }
  const bottom = () => list().scrollHeight - list().clientHeight

  it('at the bottom: a reply that grows later (its link card unfurling) keeps the panel at the bottom', async () => {
    await show()
    expect(rows(list())).toHaveLength(20)
    expect(list().scrollTop).toBe(bottom())
    grown.set('R0020', 240) // the newest reply's card arrives
    frame()
    expect(list().scrollTop).toBe(bottom())
    grown.set('R0019', 60) // one above it gets a reaction row
    frame()
    expect(list().scrollTop).toBe(bottom())
  })

  it('scrolled up: a reply growing leaves the view where it is', async () => {
    await show()
    list().scrollTop -= 500
    frame()
    const at = list().scrollTop
    grown.set('R0020', 240)
    frame()
    expect(list().scrollTop).toBe(at)
  })

  it('a little above the bottom (within the threshold) still counts as at the bottom', async () => {
    await show()
    list().scrollTop -= 20
    frame()
    grown.set('R0020', 240)
    frame()
    expect(list().scrollTop).toBe(bottom())
  })

  it('back at the bottom after scrolling up: pinned again', async () => {
    await show()
    list().scrollTop -= 500
    frame()
    list().scrollTop = bottom()
    frame()
    grown.set('R0020', 240)
    frame()
    expect(list().scrollTop).toBe(bottom())
  })

  it('closing the panel lets its observers go', async () => {
    await show()
    expect([...observers].some((o) => o.els.has(list()))).toBe(true)
    unmount(cmp!)
    cmp = undefined
    expect([...observers].some((o) => o.els.size)).toBe(false)
  })
})
