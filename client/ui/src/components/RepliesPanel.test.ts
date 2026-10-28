// @vitest-environment jsdom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Message } from '../lib/protocol/Message'
import { FakeIntersectionObserver, fakeMatchMedia } from '../lib/testMotion'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))
vi.stubGlobal('IntersectionObserver', FakeIntersectionObserver)
vi.stubGlobal('matchMedia', fakeMatchMedia(false).matchMedia)

import RepliesPanel from './RepliesPanel.svelte'
import { app, handleReady, resetState, ui } from '../lib/store.svelte'
import { addHistory, applyEvent, markHistory, setPendingStatus } from '../lib/state'
import { windowFocus } from '../lib/windowFocus.svelte'
import { positionAt } from '../lib/composerDom'
import { testMe, testReady } from '../lib/testReady'

const sam = { id: 'U2', username: 'sam', avatar_hash: null }
const ready = testReady([{ id: 'C1', server_id: 'S1', kind: 'quotes', name: 'quotes', position: 0 }], {
  members: [{ server_id: 'S1', members: [{ user: testMe, nickname: null }, { user: sam, nickname: null }] }],
  people: [{ user: testMe, online: true, last_seen_at: null }, { user: sam, online: true, last_seen_at: null }],
})

const base = (id: string): Message => ({
  id, channel_id: 'C1', author_id: 'U2', kind: 'normal', content: '', reply_to_id: null, created_at: '2026-10-01T10:00:00.000Z',
  edited_at: null, deleted: false, mentions: [], reactions: [], thread_root_id: null, quote: null, qoty_card: null,
})
const root: Message = {
  ...base('Q1'), content: '“hi” — sam',
  quote: {
    lines: [{ id: 'L1', speaker_id: 'U2', speaker_name: null, text: 'hi', media: null, caption: null, awards: 0, awarders: [], awarded: false }],
    year: 2026, said_on: null, reply_count: 2, last_reply_id: 'R0002', replies_seen: null, wins: [],
  },
}
const reply = (n: number, over: Partial<Message> = {}): Message => ({ ...base(`R${String(n).padStart(4, '0')}`), content: `reply ${n}`, thread_root_id: 'Q1', ...over })

let pages: Record<string, Message[]> = {}
let cmp: ReturnType<typeof mount> | undefined
const onClose = vi.fn()
const settle = () => new Promise((r) => setTimeout(r, 0))
const calls = (cmd: string) => invoke.mock.calls.filter((c) => c[0] === cmd)
const panel = () => document.querySelector<HTMLElement>('.replies-panel')!

async function show(q: Message = root) {
  app.state = markHistory(addHistory({ ...app.state }, 'C1', [q]), 'C1', true)
  cmp = mount(RepliesPanel, { target: document.body, props: { root: q, serverId: 'S1', onClose } })
  flushSync()
  await settle()
  flushSync()
}
function type(text: string) {
  const box = panel().querySelector<HTMLElement>('[role="textbox"]')!
  box.focus()
  box.replaceChildren(document.createTextNode(text))
  const p = positionAt(box, text.length)
  const r = document.createRange()
  r.setStart(p.node, p.offset)
  r.collapse(true)
  getSelection()!.removeAllRanges()
  getSelection()!.addRange(r)
  box.dispatchEvent(new InputEvent('input', { bubbles: true }))
  flushSync()
  return box
}
const press = (el: Element, key: string) => {
  el.dispatchEvent(new KeyboardEvent('keydown', { key, bubbles: true, cancelable: true }))
  flushSync()
}

beforeEach(() => {
  Element.prototype.scrollTo = () => {}
  localStorage.clear()
  resetState()
  handleReady(ready)
  pages = { '': [reply(2), reply(1)] } // newest first, as the server sends them
  invoke.mockReset()
  invoke.mockImplementation(async (cmd: string, args?: unknown) => {
    if (cmd === 'list_replies') return pages[(args as { before: string | null }).before ?? ''] ?? []
    return undefined
  })
  onClose.mockClear()
  windowFocus.focused = true
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})

describe('RepliesPanel', () => {
  it('pins the quote and lists replies', async () => {
    await show()
    expect(calls('list_replies')[0][1]).toEqual({ messageId: 'Q1', before: null })
    const pinned = panel().querySelector('.quote')!
    expect(pinned.querySelector('.line .text')?.textContent).toBe('“hi”')
    expect(pinned.querySelector('button[aria-label="Reply"]')).toBeNull() // no Reply on the pinned quote
    expect(pinned.querySelector('button.replies')).toBeNull() // nor "View N replies": they're right here
    const texts = [...panel().querySelectorAll('.msg .content')].map((e) => e.textContent?.trim())
    expect(texts).toEqual(['reply 1', 'reply 2'])
    // replies keep normal message actions (reply within the thread, react)
    expect(panel().querySelectorAll('.msg button[aria-label="Reply"]')).toHaveLength(2)
  })

  it('sends with thread_root_id', async () => {
    await show()
    press(type('hello'), 'Enter')
    const [, args] = calls('send_reply')[0] as [string, { channelId: string; rootId: string; content: string; replyToId: string | null; nonce: string }]
    expect(args).toMatchObject({ channelId: 'C1', rootId: 'Q1', content: 'hello', replyToId: null })
    // the unsent reply waits in the panel (not the channel list), tied to its quote
    expect(app.state.pending['C1']).toEqual([expect.objectContaining({ nonce: args.nonce, thread_root_id: 'Q1', reply_to_id: null })])
    expect(panel().querySelector('.pend')?.textContent).toContain('Sending…')

    // a failed one retries as a reply (not a channel message)
    app.state = setPendingStatus(app.state, 'C1', args.nonce, 'failed')
    flushSync()
    ;[...panel().querySelectorAll<HTMLButtonElement>('.pend button')].find((b) => b.textContent === 'Retry')!.click()
    flushSync()
    expect(calls('send_reply')).toHaveLength(2)
    expect(calls('send_reply')[1][1]).toMatchObject({ rootId: 'Q1', content: 'hello', replyToId: null })
    expect(calls('send_message')).toHaveLength(0)
  })

  it('replying to a reply keeps reply_to_id inside the thread', async () => {
    await show()
    panel().querySelectorAll<HTMLButtonElement>('.msg button[aria-label="Reply"]')[0].click()
    flushSync()
    expect(panel().querySelector('.replying')?.textContent).toContain('sam')
    press(type('same'), 'Enter')
    expect(calls('send_reply')[0][1]).toMatchObject({ rootId: 'Q1', replyToId: 'R0001' })
    expect(panel().querySelector('.replying')).toBeNull()
  })

  it('marks seen on open', async () => {
    await show()
    expect(calls('mark_replies_seen')).toEqual([['mark_replies_seen', { rootId: 'Q1', messageId: 'R0002' }]])
    // a new reply while it's open and focused: seen too
    app.state = applyEvent(app.state, { t: 'MessageCreated', d: { message: reply(3), nonce: null } }, Date.now())
    flushSync()
    expect(calls('mark_replies_seen').at(-1)).toEqual(['mark_replies_seen', { rootId: 'Q1', messageId: 'R0003' }])
    // not while the window is unfocused; it catches up on focus
    windowFocus.focused = false
    app.state = applyEvent(app.state, { t: 'MessageCreated', d: { message: reply(4), nonce: null } }, Date.now())
    flushSync()
    expect(calls('mark_replies_seen')).toHaveLength(2)
    windowFocus.focused = true
    flushSync()
    expect(calls('mark_replies_seen').at(-1)).toEqual(['mark_replies_seen', { rootId: 'Q1', messageId: 'R0004' }])
  })

  it("M17: a non-member's open panel never marks seen (the server would refuse every time)", async () => {
    handleReady({ ...ready, members: [{ server_id: 'S1', members: [{ user: sam, nickname: null }] }] })
    await show()
    app.state = applyEvent(app.state, { t: 'MessageCreated', d: { message: reply(3), nonce: null } }, Date.now())
    flushSync()
    expect(calls('mark_replies_seen')).toHaveLength(0)
  })

  it('M24: retrying a failed reply keeps the reply target you just picked', async () => {
    await show()
    press(type('first'), 'Enter')
    const nonce = (calls('send_reply')[0][1] as { nonce: string }).nonce
    app.state = setPendingStatus(app.state, 'C1', nonce, 'failed')
    flushSync()
    panel().querySelectorAll<HTMLButtonElement>('.msg:not(.unsent) button[aria-label="Reply"]')[0].click() // pick R0001
    flushSync()
    ;[...panel().querySelectorAll<HTMLButtonElement>('.pend button')].find((b) => b.textContent === 'Retry')!.click()
    flushSync()
    expect(calls('send_reply')[1][1]).toMatchObject({ content: 'first', replyToId: null }) // the retry keeps its own target
    expect(panel().querySelector('.replying')?.textContent).toContain('sam') // and yours stays picked
    press(type('second'), 'Enter')
    expect(calls('send_reply')[2][1]).toMatchObject({ content: 'second', replyToId: 'R0001' })
  })

  it('does not mark what was already seen', async () => {
    await show({ ...root, quote: { ...root.quote!, replies_seen: 'R0002' } })
    expect(calls('mark_replies_seen')).toHaveLength(0)
  })

  it('Escape closes', async () => {
    await show()
    press(panel().querySelector('[role="textbox"]')!, 'Escape')
    expect(onClose).toHaveBeenCalledTimes(1)
  })

  it('Escape that cancels an inline reply does not close the panel', async () => {
    await show()
    panel().querySelectorAll<HTMLButtonElement>('.msg button[aria-label="Reply"]')[0].click()
    flushSync()
    press(panel().querySelector('[role="textbox"]')!, 'Escape')
    expect(panel().querySelector('.replying')).toBeNull()
    expect(onClose).not.toHaveBeenCalled()
  })

  it('Escape in a field outside the panel is left to that field', async () => {
    await show()
    const outside = document.createElement('textarea')
    document.body.append(outside)
    outside.focus()
    press(outside, 'Escape')
    expect(onClose).not.toHaveBeenCalled()
  })

  it('Escape is left to an open dialog, but not to one hidden by CSS', async () => {
    await show()
    const dlg = document.createElement('div')
    dlg.setAttribute('role', 'dialog')
    document.body.append(dlg)
    press(panel().querySelector('[role="textbox"]')!, 'Escape')
    expect(onClose).not.toHaveBeenCalled()
    dlg.style.display = 'none'
    press(panel().querySelector('[role="textbox"]')!, 'Escape')
    expect(onClose).toHaveBeenCalledTimes(1)
    dlg.remove()
  })

  it('pages older replies on scroll up and stops at the start', async () => {
    const full = Array.from({ length: 50 }, (_, i) => reply(100 - i)) // R0100..R0051, a full page
    pages = { '': full, R0051: [reply(50), reply(49)] }
    await show({ ...root, quote: { ...root.quote!, reply_count: 52, last_reply_id: 'R0100' } })
    const list = panel().querySelector<HTMLElement>('.list')!
    list.dispatchEvent(new Event('scroll'))
    await settle()
    flushSync()
    expect(calls('list_replies').at(-1)![1]).toEqual({ messageId: 'Q1', before: 'R0051' })
    expect(app.state.threads['Q1'][0].id).toBe('R0049')
    expect(app.state.threadStart['Q1']).toBe(true)
    list.dispatchEvent(new Event('scroll'))
    await settle()
    expect(calls('list_replies')).toHaveLength(2) // the start is reached
  })

  it('refetches after a reconnect', async () => {
    await show()
    expect(calls('list_replies')).toHaveLength(1)
    handleReady(ready) // threads are cleared on Ready
    flushSync()
    await settle()
    flushSync()
    expect(calls('list_replies')).toHaveLength(2)
    expect(panel().querySelectorAll('.msg')).toHaveLength(2)
  })

  it('says which thread is open while mounted', async () => {
    await show()
    expect(ui.openThreadId).toBe('Q1')
    unmount(cmp!)
    cmp = undefined
    expect(ui.openThreadId).toBeNull()
  })

  it('M13/M16: holds its quote while open, and lets the thread go when it closes', async () => {
    await show()
    expect(app.state.openRoot?.id).toBe('Q1')
    expect(app.state.threadStart['Q1']).toBe(true)
    unmount(cmp!)
    cmp = undefined
    app.state = { ...app.state, pending: { C1: [] } } // a change in the same flush survives the close
    await settle()
    expect(app.state.openRoot).toBeNull()
    expect(app.state.threads['Q1']).toBeUndefined()
    expect(app.state.threadStart['Q1']).toBeUndefined()
    expect(app.state.pending['C1']).toEqual([])
  })
})
