// @vitest-environment jsdom
// Opening cheaply (#238): a channel opens with its newest messages, enough to fill the list, and draws the rest of its
// loaded history after that first frame, a frame's worth at a time, keeping the view where it was from the bottom. jsdom has no layout, so
// each message is 100px tall here and the list's height is set per test.
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
let listHeight = 1000
const tops = new WeakMap<Element, number>()
const isList = (el: Element) => el.classList.contains('list')
const drawn = () => [...document.querySelectorAll('.list [data-message-id]')].map((e) => (e as HTMLElement).dataset.messageId)
const afterFrame = () => new Promise<void>((r) => requestAnimationFrame(() => setTimeout(() => setTimeout(r))))
const list = () => document.querySelector<HTMLElement>('.list')!

let cmp: ReturnType<typeof mount> | undefined
const saved: [object, string, PropertyDescriptor | undefined][] = []
function fake(proto: object, key: string, desc: PropertyDescriptor) {
  saved.push([proto, key, Object.getOwnPropertyDescriptor(proto, key)])
  Object.defineProperty(proto, key, { configurable: true, ...desc })
}

describe('TextChannel: drawn incrementally', () => {
  beforeEach(() => {
    fake(Element.prototype, 'scrollHeight', { get(this: Element) { return isList(this) ? this.querySelectorAll('[data-message-id]').length * ROW : 0 } })
    fake(Element.prototype, 'clientHeight', { get(this: Element) { return isList(this) ? listHeight : 0 } })
    fake(Element.prototype, 'scrollTop', {
      get(this: Element) { return tops.get(this) ?? 0 },
      set(this: Element, v: number) { tops.set(this, Math.max(0, Math.min(v, this.scrollHeight - this.clientHeight))) },
    })
    // a message's place on screen: its row in the list, less the list's scroll
    fake(Element.prototype, 'getBoundingClientRect', { value(this: Element) {
      const l = this.closest('.list')
      const i = l ? [...l.querySelectorAll('[data-message-id]')].indexOf(this) : -1
      const top = i < 0 || !l ? 0 : i * ROW - l.scrollTop
      return { top, bottom: top + ROW, left: 0, right: 0, width: 0, height: i < 0 ? 0 : ROW, x: 0, y: top, toJSON() {} } as DOMRect
    } })
    Element.prototype.scrollTo = function (this: Element, o?: ScrollToOptions | number) { if (typeof o === 'object' && o.top !== undefined) this.scrollTop = o.top }
    Element.prototype.scrollIntoView = () => {}
    resetState()
    ui.jump = null
    listHeight = 1000
    app.state = loadLatest(applyReady(emptyState(), testReady([{ id: 'C1', server_id: 'S1', kind: 'text', name: 'general', position: 0 }])), 'C1', Array.from({ length: 60 }, (_, i) => msg(i + 1)))
  })
  afterEach(() => {
    if (cmp) unmount(cmp)
    cmp = undefined
    document.body.innerHTML = ''
    for (const [proto, key, desc] of saved.reverse()) desc ? Object.defineProperty(proto, key, desc) : delete (proto as Record<string, unknown>)[key]
    saved.length = 0
  })
  const show = async () => {
    cmp = mount(TextChannel, { target: document.body, props: { channelId: 'C1', serverId: 'S1' } })
    flushSync()
    await tick()
    flushSync()
  }

  it('opens with the newest that fill the list at the bottom, then draws the rest a frame at a time keeping the view still', async () => {
    await show()
    expect(drawn()).toHaveLength(24)
    expect(drawn().at(-1)).toBe('M0060')
    expect(document.querySelector('.start')).toBeNull() // the start of the channel isn't reached yet on screen
    // scrolled to the bottom of what's drawn
    expect(list().scrollTop).toBe(24 * ROW - listHeight)
    list().scrollTop -= 150 // a little way up before the rest arrives
    // a frame's worth at a time, the view staying the same distance from the bottom
    await afterFrame()
    expect(drawn()).toHaveLength(48)
    expect(list().scrollHeight - list().scrollTop).toBe(listHeight + 150)
    await afterFrame()
    expect(drawn()).toHaveLength(60)
    expect(drawn()[0]).toBe('M0001')
    expect(list().scrollHeight - list().scrollTop).toBe(listHeight + 150)
  })

  it('too few to fill the list: as many more as should, once', async () => {
    listHeight = 3000 // 24 rows are 2400px
    await show()
    expect(drawn()).toHaveLength(Math.ceil((24 * 3200) / 2400))
    await afterFrame()
    await afterFrame()
    expect(drawn()).toHaveLength(60)
  })

  it("everything at once when even those don't fill it", async () => {
    listHeight = 20000
    await show()
    expect(drawn()).toHaveLength(60)
  })

  it('a short history is drawn whole', async () => {
    app.state = loadLatest(app.state, 'C1', Array.from({ length: 10 }, (_, i) => msg(i + 1)))
    await show()
    expect(drawn()).toHaveLength(10)
    expect(document.querySelector('.start')).not.toBeNull()
  })

  it('a jump to one of the rest draws everything first and flashes it', async () => {
    await show()
    expect(drawn()).toHaveLength(24)
    ui.jump = { channelId: 'C1', messageId: 'M0002', seq: 1 }
    for (let i = 0; i < 3; i++) await tick()
    flushSync()
    expect(drawn()).toHaveLength(60)
    expect(document.querySelector('[data-message-id="M0002"]')!.classList.contains('jump-flash')).toBe(true)
  })

  it('opened by a jump: drawn whole from the start', async () => {
    ui.jump = { channelId: 'C1', messageId: 'M0002', seq: 1 }
    await show()
    expect(drawn()).toHaveLength(60)
  })
})
