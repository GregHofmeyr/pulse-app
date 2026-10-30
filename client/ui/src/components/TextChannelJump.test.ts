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
import { windowFocus } from '../lib/windowFocus.svelte'
import { addPending, applyEvent, MISSED_CAP, applyReady, emptyState, loadLatest, loadWindow } from '../lib/state'
import { testReady } from '../lib/testReady'

const msg = (n: number): Message => ({
  id: `M${String(n).padStart(4, '0')}`, channel_id: 'C1', author_id: 'U2', kind: 'normal', content: `m${n}`,
  reply_to_id: null, created_at: '2026-10-01T00:00:00.000Z', edited_at: null, deleted: false, mentions: [], reactions: [], thread_root_id: null, quote: null, qoty_card: null, clip: null,
})
const quote = (n: number): Message => ({
  ...msg(n), content: `“q${n}” — sam`,
  quote: { lines: [{ id: `L${n}`, speaker_id: 'U2', speaker_name: null, text: `q${n}`, media: null, caption: null, awards: 0, awarders: [], awarded: false }], year: 2026, said_on: null, reply_count: 0, last_reply_id: null, replies_seen: null, wins: [] },
})
const newestFirst = (from: number, n: number, f = msg) => Array.from({ length: n }, (_, i) => f(from + i)).reverse()
const ready = (kind: 'text' | 'quotes') => applyReady(emptyState(), testReady([{ id: 'C1', server_id: 'S1', kind, name: 'general', position: 0 }]))
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
    ui.openChannelId = 'C1'
    windowFocus.focused = true
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
    app.state = loadLatest(ready('text'), 'C1', newestFirst(900, 50))
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

  it('a failed jump says why', async () => {
    app.state = loadWindow(ready('text'), 'C1', newestFirst(900, 50), 'M0949')
    show()
    ui.jump = { channelId: 'C1', messageId: 'M0001', seq: 1, error: "can't reach the server: offline" }
    await settle()
    expect(document.querySelector('[role=status].jump-notice')?.textContent).toBe("can't reach the server: offline")
  })

  it('a jump left unhandled goes with its channel view', async () => {
    app.state = loadWindow(ready('text'), 'C1', newestFirst(100, 50), 'M0125')
    show()
    ui.jump = { channelId: 'C1', messageId: 'M9999', seq: 1 } // not loaded (yet)
    await settle()
    unmount(cmp!)
    cmp = undefined
    expect(ui.jump).toBeNull()
  })

  it("looking away from a detached window doesn't move the NEW line onto read messages", async () => {
    const read = { read_states: [{ channel_id: 'C1', last_read_message_id: 'M0949', unread: 0, mentions: 0 }] }
    app.state = loadWindow(applyReady(emptyState(), testReady([{ id: 'C1', server_id: 'S1', kind: 'text', name: 'general', position: 0 }], read)), 'C1', newestFirst(100, 50), 'M0125')
    invoke.mockImplementation(async (cmd, args) => (cmd === 'list_messages' && !(args as { before: string | null }).before ? newestFirst(900, 50) : undefined))
    show()
    windowFocus.focused = false // you look away
    flushSync()
    ;[...document.querySelectorAll('button')].find((b) => b.textContent === 'Jump to present')!.click()
    await settle()
    expect(app.state.messages['C1'][0].id).toBe('M0900')
    expect(document.querySelector('.newline')).toBeNull() // everything up to M0949 was read
  })

  it('a message sent from a detached window shows once the present is loaded', async () => {
    let releaseLatest: (p: Message[]) => void = () => {}
    const mine = { ...msg(2000), author_id: 'U1', content: 'hello' }
    invoke.mockImplementation((cmd, args) =>
      cmd === 'list_messages' && !(args as { before: string | null }).before ? new Promise((done) => (releaseLatest = done as never))
        : cmd === 'list_messages_after' ? Promise.resolve([mine]) : Promise.resolve(undefined))
    app.state = loadWindow(ready('text'), 'C1', newestFirst(100, 50), 'M0125')
    app.state = addPending(app.state, 'C1', { nonce: 'n1', content: 'hello', reply_to_id: null, thread_root_id: null, status: 'failed', created_at: '2026-10-01T00:00:00.000Z' })
    show()
    ;[...document.querySelectorAll('button')].find((b) => b.textContent === 'Retry')!.click() // sends it again
    await settle()
    const nonce = (invoke.mock.calls.find(([c]) => c === 'send_message')![1] as { nonce: string }).nonce
    // confirmed live before the latest page answers, and the page was read before it
    app.state = applyEvent(app.state, { t: 'MessageCreated', d: { message: mine, nonce } }, 0)
    releaseLatest(newestFirst(900, 50))
    await settle()
    expect(app.state.messages['C1'].at(-1)!.id).toBe('M2000')
    expect(app.state.history['C1'].end).toBe(true)
    expect(document.querySelector('[data-message-id="M2000"]')).toBeTruthy()
  })

  it('I4: a quote posted from a detached window shows once the present is loaded', async () => {
    const posted = { ...quote(2000), author_id: 'U1' }
    invoke.mockImplementation(async (cmd, args) =>
      cmd === 'post_quote' ? posted
        : cmd === 'list_messages' && !(args as { before: string | null }).before ? [posted, ...newestFirst(900, 49, quote)]
          : undefined)
    app.state = loadWindow(ready('quotes'), 'C1', newestFirst(100, 50, quote), 'M0125')
    show()
    const text = document.querySelector<HTMLTextAreaElement>('textarea.qtext')!
    text.value = 'back in my day'; text.dispatchEvent(new Event('input', { bubbles: true })); flushSync()
    const who = document.querySelector<HTMLInputElement>('input.who')!
    who.focus(); who.value = 'Grandpa'; who.dispatchEvent(new Event('input', { bubbles: true })); flushSync()
    const use = [...document.querySelectorAll<HTMLElement>('[role=option]')].find((o) => o.textContent?.includes('Grandpa'))!
    use.dispatchEvent(new MouseEvent('mousedown', { bubbles: true, cancelable: true })); use.click(); flushSync()
    text.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true }))
    // the gateway's copy lands while detached: it waits in `missed`
    app.state = applyEvent(app.state, { t: 'MessageCreated', d: { message: posted, nonce: null } }, 0)
    await settle()
    await settle()
    expect(invoke.mock.calls.some(([c]) => c === 'post_quote')).toBe(true)
    expect(invoke).toHaveBeenCalledWith('list_messages', { channelId: 'C1', before: null })
    expect(app.state.history['C1'].end).toBe(true)
    expect(document.querySelector('.quote[data-message-id="M2000"]')).toBeTruthy()
  })

  it('a live message that lands while a forward page loads is fetched next', async () => {
    let releaseAfter: (p: Message[]) => void = () => {}
    let afters = 0
    invoke.mockImplementation((cmd) =>
      cmd !== 'list_messages_after' ? Promise.resolve(undefined)
        : afters++ === 0 ? new Promise((done) => (releaseAfter = done as never)) : Promise.resolve([msg(2000)]))
    app.state = loadWindow(ready('text'), 'C1', newestFirst(100, 50), 'M0125')
    show()
    document.querySelector('.list')!.dispatchEvent(new Event('scroll'))
    // M2000 arrives live after the server answered, before the page is applied
    app.state = applyEvent(app.state, { t: 'MessageCreated', d: { message: msg(2000), nonce: null } }, 0)
    releaseAfter(newestFirst(150, 10))
    await settle()
    await settle()
    expect(invoke.mock.calls.filter(([c]) => c === 'list_messages_after').map(([, a]) => a)).toEqual([
      { channelId: 'C1', messageId: 'M0149' },
      { channelId: 'C1', messageId: 'M0159' },
    ])
    expect(app.state.messages['C1'].at(-1)!.id).toBe('M2000')
    expect(app.state.history['C1'].end).toBe(true)
  })

  const afterCalls = () => invoke.mock.calls.filter(([c]) => c === 'list_messages_after')
  const pageForward = async () => {
    show()
    document.querySelector('.list')!.dispatchEvent(new Event('scroll')) // jsdom: always at the bottom
    for (let i = 0; i < 6; i++) await settle()
  }

  it('an empty forward page with a missed message nothing can fetch retries once, then reaches the latest', async () => {
    app.state = loadWindow(ready('text'), 'C1', newestFirst(100, 50), 'M0125')
    // a missed message nothing can fetch (only a hard delete event could tell us it's gone)
    app.state = applyEvent(app.state, { t: 'MessageCreated', d: { message: msg(2000), nonce: null } }, 0)
    invoke.mockImplementation(async (cmd) => (cmd === 'list_messages_after' ? [] : undefined))
    await pageForward()
    expect(afterCalls()).toHaveLength(2) // one retry, never a loop
    expect(app.state.history['C1']).toEqual({ loaded: true, start: false, end: true })
    expect([...document.querySelectorAll('button')].some((b) => b.textContent === 'Jump to present')).toBe(false)
  })

  it('a live message that arrives during the empty retry is not lost', async () => {
    app.state = loadWindow(ready('text'), 'C1', newestFirst(100, 50), 'M0125')
    app.state = applyEvent(app.state, { t: 'MessageCreated', d: { message: msg(2000), nonce: null } }, 0)
    let afters = 0
    invoke.mockImplementation(async (cmd) => {
      if (cmd !== 'list_messages_after') return undefined
      if (afters++ === 1) app.state = applyEvent(app.state, { t: 'MessageCreated', d: { message: msg(3000), nonce: null } }, 0) // during the retry
      return afters <= 3 ? [] : [msg(3000)]
    })
    await pageForward()
    expect(afterCalls().length).toBeLessThanOrEqual(5) // bounded
    expect(app.state.messages['C1'].some((m) => m.id === 'M3000')).toBe(true) // no gap
    expect(app.state.history['C1'].end).toBe(true)
  })

  it('an empty forward page with older missed messages dropped asks once more, then reaches the latest', async () => {
    app.state = loadWindow(ready('text'), 'C1', newestFirst(100, 50), 'M0125')
    const live = Array.from({ length: MISSED_CAP + 1 }, (_, i) => 2000 + i)
    for (const n of live) app.state = applyEvent(app.state, { t: 'MessageCreated', d: { message: msg(n), nonce: null } }, 0)
    for (const n of live.slice(1)) app.state = applyEvent(app.state, { t: 'MessageDeleted', d: { channel_id: 'C1', message_id: `M${n}`, author_id: 'U2', mentions: [], thread_root_id: null } }, 0)
    expect(app.state.history['C1'].missedMore).toBe(true)
    invoke.mockImplementation(async (cmd) => (cmd === 'list_messages_after' ? [] : undefined))
    await pageForward()
    expect(afterCalls()).toHaveLength(2)
    expect(app.state.history['C1']).toEqual({ loaded: true, start: false, end: true })
  })

  it('the retry brings the missed message in when the server has it by then', async () => {
    app.state = loadWindow(ready('text'), 'C1', newestFirst(100, 50), 'M0125')
    app.state = applyEvent(app.state, { t: 'MessageCreated', d: { message: msg(2000), nonce: null } }, 0)
    let afters = 0
    invoke.mockImplementation(async (cmd) => (cmd !== 'list_messages_after' ? undefined : afters++ === 0 ? [] : [msg(2000)]))
    await pageForward()
    expect(afterCalls()).toHaveLength(2)
    expect(app.state.messages['C1'].at(-1)!.id).toBe('M2000')
    expect(app.state.history['C1'].end).toBe(true)
  })

  it('Jump to present attaches after a missed message was deleted', async () => {
    app.state = loadWindow(ready('text'), 'C1', newestFirst(100, 50), 'M0125')
    app.state = applyEvent(app.state, { t: 'MessageCreated', d: { message: msg(2000), nonce: null } }, 0)
    app.state = applyEvent(app.state, { t: 'MessageDeleted', d: { channel_id: 'C1', message_id: 'M2000', author_id: 'U2', mentions: [], thread_root_id: null } }, 0)
    invoke.mockImplementation(async (cmd, args) => (cmd === 'list_messages' && !(args as { before: string | null }).before ? newestFirst(900, 50) : cmd === 'list_messages_after' ? [] : undefined))
    show()
    ;[...document.querySelectorAll('button')].find((b) => b.textContent === 'Jump to present')!.click()
    await settle()
    expect(app.state.history['C1'].end).toBe(true)
    expect(invoke.mock.calls.filter(([c]) => c === 'list_messages_after')).toHaveLength(0)
    expect([...document.querySelectorAll('button')].some((b) => b.textContent === 'Jump to present')).toBe(false)
  })
})

describe("TextChannel: a notification's reply opens its quote's replies", () => {
  beforeEach(() => {
    Element.prototype.scrollTo = () => {}
    Element.prototype.scrollIntoView = () => {}
    resetState()
    ui.jump = null
    ui.openReplies = null
    invoke.mockReset()
    invoke.mockImplementation(async (cmd: string) => (cmd === 'list_replies' ? [] : undefined))
    ui.openChannelId = 'C1'
    windowFocus.focused = true
  })
  afterEach(() => {
    if (cmp) unmount(cmp)
    cmp = undefined
    document.body.innerHTML = ''
  })

  it('a loaded quote: its panel opens and the request is cleared', async () => {
    app.state = loadLatest(ready('quotes'), 'C1', newestFirst(1, 5, quote))
    show()
    ui.openReplies = { channelId: 'C1', rootId: 'M0003' }
    await settle()
    expect(document.querySelector('.replies-panel .quote')?.getAttribute('data-message-id')).toBe('M0003')
    expect(ui.openReplies).toBeNull()
  })

  it('a quote not loaded: its window is fetched, then the panel opens', async () => {
    app.state = loadLatest(ready('quotes'), 'C1', newestFirst(100, 5, quote))
    invoke.mockImplementation(async (cmd: string) =>
      cmd === 'list_messages_around' ? newestFirst(1, 5, quote) : cmd === 'list_replies' ? [] : undefined)
    show()
    ui.openReplies = { channelId: 'C1', rootId: 'M0003' }
    await settle()
    await settle()
    expect(invoke).toHaveBeenCalledWith('list_messages_around', expect.objectContaining({ channelId: 'C1', messageId: 'M0003' }))
    expect(document.querySelector('.replies-panel .quote')?.getAttribute('data-message-id')).toBe('M0003')
    expect(ui.openReplies).toBeNull()
  })

  it("another channel's request is left alone", async () => {
    app.state = loadLatest(ready('quotes'), 'C1', newestFirst(1, 5, quote))
    show()
    ui.openReplies = { channelId: 'C2', rootId: 'M0003' }
    await settle()
    expect(document.querySelector('.replies-panel')).toBeNull()
    expect(ui.openReplies).toEqual({ channelId: 'C2', rootId: 'M0003' })
  })
})
