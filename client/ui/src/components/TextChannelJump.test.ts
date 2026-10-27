// @vitest-environment jsdom
// Jumping to a message (lib/jump.ts sets the request; the channel view scrolls, flashes and pages).
import { flushSync, mount, tick, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Message } from '../lib/protocol/Message'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: unknown) => invoke(cmd, args) }))

import TextChannel from './TextChannel.svelte'
import { app, resetState, ui } from '../lib/store.svelte'
import { motion } from '../lib/animate.svelte'
import { applyReady, emptyState, loadWindow } from '../lib/state'

const msg = (n: number): Message => ({
  id: `M${String(n).padStart(4, '0')}`, channel_id: 'C1', author_id: 'U2', kind: 'normal', content: `m${n}`,
  reply_to_id: null, created_at: '2026-10-01T00:00:00.000Z', edited_at: null, deleted: false, mentions: [], reactions: [], thread_root_id: null, quote: null, qoty_card: null,
})
const quote = (n: number): Message => ({
  ...msg(n), content: `“q${n}” — sam`,
  quote: { lines: [{ id: `L${n}`, speaker_id: 'U2', speaker_name: null, text: `q${n}`, media: null, caption: null, awards: 0, awarders: [], awarded: false }], year: 2026, said_on: null, reply_count: 0, last_reply_id: null, replies_seen: null, wins: [] },
})
const newestFirst = (from: number, n: number, f = msg) => Array.from({ length: n }, (_, i) => f(from + i)).reverse()
const me = { id: 'U1', username: 'alex', avatar_hash: null }
const ready = (kind: 'text' | 'quotes') => applyReady(emptyState(), {
  me, servers: [{ id: 'S1', name: 'Main', icon_hash: null }],
  channels: [{ id: 'C1', server_id: 'S1', kind, name: 'general', position: 0 }],
  members: [{ server_id: 'S1', members: [{ user: me, nickname: null }] }],
  dm_members: [], voice: [], people: [], read_states: [], mutes: [], hidden: [], latest: [], streams: [], custom_emoji: [], frequent_emoji: [], qoty: [], speaker_links: [], thread_seen: [],
})
const settle = async () => {
  for (let i = 0; i < 5; i++) await tick()
  flushSync()
}

let cmp: ReturnType<typeof mount> | undefined
const show = () => {
  cmp = mount(TextChannel, { target: document.body, props: { channelId: 'C1', serverId: 'S1' } })
  flushSync()
}
const scrolled: Element[] = []

describe('TextChannel: jump to a message', () => {
  beforeEach(() => {
    Element.prototype.scrollTo = () => {}
    Element.prototype.scrollIntoView = function (this: Element) { scrolled.push(this) }
    scrolled.length = 0
    resetState()
    ui.jump = null
    motion.reducedMotion = false
    invoke.mockReset()
    invoke.mockImplementation(async () => undefined)
  })
  afterEach(() => {
    if (cmp) unmount(cmp)
    cmp = undefined
    document.body.innerHTML = ''
    vi.useRealTimers()
  })

  it('the jump target is scrolled to and flashes', async () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] })
    app.state = loadWindow(ready('text'), 'C1', newestFirst(100, 50), 'M0125')
    show()
    ui.jump = { channelId: 'C1', messageId: 'M0125', seq: 1 }
    await settle()
    const target = document.querySelector('[data-message-id="M0125"]')!
    expect(target).toBeTruthy()
    expect(scrolled).toContain(target)
    expect(target.classList.contains('jump-flash')).toBe(true)
    expect(target.classList.contains('still')).toBe(false)
    expect(ui.jump).toBeNull() // handled
    vi.advanceTimersByTime(2100)
    expect(target.classList.contains('jump-flash')).toBe(false)
    // the same target again flashes again
    ui.jump = { channelId: 'C1', messageId: 'M0125', seq: 2 }
    await settle()
    expect(target.classList.contains('jump-flash')).toBe(true)
  })

  it('jumps to a quote in a quotes channel', async () => {
    app.state = loadWindow(ready('quotes'), 'C1', newestFirst(100, 50, quote), 'M0125')
    show()
    ui.jump = { channelId: 'C1', messageId: 'M0125', seq: 1 }
    await settle()
    const target = document.querySelector('.quote[data-message-id="M0125"]')!
    expect(scrolled).toContain(target)
    expect(target.classList.contains('jump-flash')).toBe(true)
  })

  it('no flash animation under reduced motion', async () => {
    motion.reducedMotion = true
    app.state = loadWindow(ready('text'), 'C1', newestFirst(100, 50), 'M0125')
    show()
    ui.jump = { channelId: 'C1', messageId: 'M0125', seq: 1 }
    await settle()
    const target = document.querySelector('[data-message-id="M0125"]')!
    expect(target.classList.contains('jump-flash')).toBe(true)
    expect(target.classList.contains('still')).toBe(true) // a static outline, no animation
  })

  it("a message that can't be found says so", async () => {
    app.state = loadWindow(ready('text'), 'C1', newestFirst(900, 50))
    show()
    ui.jump = { channelId: 'C1', messageId: 'M0001', seq: 1, missing: true }
    await settle()
    expect(document.querySelector('[role=status].jump-notice')?.textContent).toBe("Couldn't find that message")
    expect(ui.jump).toBeNull()
  })

  it('Jump to present reloads the latest', async () => {
    app.state = loadWindow(ready('text'), 'C1', newestFirst(100, 50), 'M0125')
    invoke.mockImplementation(async (cmd, args) => (cmd === 'list_messages' && !(args as { before: string | null }).before ? newestFirst(900, 50) : undefined))
    show()
    const bar = () => [...document.querySelectorAll('button')].find((b) => b.textContent === 'Jump to present')
    expect(bar()).toBeTruthy()
    bar()!.click()
    await settle()
    expect(invoke).toHaveBeenCalledWith('list_messages', { channelId: 'C1', before: null })
    expect(app.state.messages['C1'][0].id).toBe('M0900')
    expect(app.state.history['C1'].end).toBe(true)
    expect(bar()).toBeUndefined()
  })

  it('the bottom of a detached window pages forward, and is not marked read', async () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] })
    app.state = loadWindow(ready('text'), 'C1', newestFirst(100, 50), 'M0125')
    invoke.mockImplementation(async (cmd) => (cmd === 'list_messages_after' ? newestFirst(150, 10) : undefined))
    show()
    vi.advanceTimersByTime(1000)
    expect(invoke.mock.calls.filter(([c]) => c === 'mark_read')).toHaveLength(0) // at the window's bottom, not the latest
    document.querySelector('.list')!.dispatchEvent(new Event('scroll')) // jsdom: always at the bottom
    await settle()
    expect(invoke).toHaveBeenCalledWith('list_messages_after', { channelId: 'C1', messageId: 'M0149' })
    expect(app.state.messages['C1'].at(-1)!.id).toBe('M0159')
    expect(app.state.history['C1'].end).toBe(true) // a short page: the latest
    flushSync()
    vi.advanceTimersByTime(1000)
    expect(invoke).toHaveBeenCalledWith('mark_read', { channelId: 'C1', messageId: 'M0159' })
  })

  it('a latest page that lands after a jump window is dropped', async () => {
    let releaseLatest: (p: Message[]) => void = () => {}
    invoke.mockImplementation((cmd) => (cmd === 'list_messages' ? new Promise((done) => (releaseLatest = done as never)) : Promise.resolve(undefined)))
    app.state = ready('text')
    show() // not loaded: fetches the latest page
    app.state = loadWindow(app.state, 'C1', newestFirst(100, 50), 'M0125') // the jump's window lands first
    releaseLatest(newestFirst(900, 50))
    await settle()
    expect(app.state.messages['C1'].map((m) => m.id)).toEqual(newestFirst(100, 50).map((m) => m.id).reverse())
  })
})
