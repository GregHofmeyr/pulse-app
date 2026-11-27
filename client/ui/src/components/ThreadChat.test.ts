// @vitest-environment jsdom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Message } from '../lib/protocol/Message'
import { FakeIntersectionObserver, fakeMatchMedia } from '../lib/testMotion'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))
vi.stubGlobal('IntersectionObserver', FakeIntersectionObserver)
vi.stubGlobal('matchMedia', fakeMatchMedia(false).matchMedia)

import ThreadChat from './ThreadChat.svelte'
import { app, handleReady, resetState, ui } from '../lib/store.svelte'
import { addHistory, markHistory } from '../lib/state'
import { windowFocus } from '../lib/windowFocus.svelte'
import { positionAt } from '../lib/composerDom'
import { testMe, testReady } from '../lib/testReady'
import { nextTask } from '../lib/testSettle'

const sam = { id: 'U2', username: 'sam', display_name: 'sam', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
const ready = testReady([{ id: 'C1', server_id: 'S1', kind: 'quotes', name: 'quotes', position: 0 }], {
  members: [{ server_id: 'S1', members: [{ user: testMe, nickname: null }, { user: sam, nickname: null }] }],
  people: [{ user: testMe, online: true, last_seen_at: null, status: 'online' }, { user: sam, online: true, last_seen_at: null, status: 'online' }],
})

const base = (id: string): Message => ({
  id, channel_id: 'C1', author_id: 'U2', kind: 'normal', content: '', reply_to_id: null, created_at: '2026-10-01T10:00:00.000Z',
  edited_at: null, deleted: false, mentions: [], reactions: [], thread_root_id: null, quote: null, qoty_card: null, clip: null, embeds: [],
})
const root: Message = {
  ...base('Q1'), content: '“hi” — sam',
  quote: {
    lines: [{ id: 'L1', speaker_id: 'U2', speaker_name: null, text: 'hi', media: null, caption: null, awards: 0, awarders: [], awarded: false }],
    year: 2026, said_on: null, reply_count: 3, last_reply_id: 'R0003', replies_seen: null, wins: [],
  },
}
const reply = (n: number, over: Partial<Message> = {}): Message => ({ ...base(`R${String(n).padStart(4, '0')}`), content: `reply ${n}`, thread_root_id: 'Q1', ...over })

let cmp: ReturnType<typeof mount> | undefined
const calls = (cmd: string) => invoke.mock.calls.filter((c) => c[0] === cmd)
const rowOf = (text: string) => [...document.querySelectorAll<HTMLElement>('.msg')].find((m) => m.querySelector('.content')?.textContent?.trim() === text)!

async function show(props: { placeholder?: string; authorTag?: string | null } = {}) {
  app.state = markHistory(addHistory({ ...app.state }, 'C1', [root]), 'C1', true)
  cmp = mount(ThreadChat, { target: document.body, props: { root, serverId: 'S1', placeholder: 'Comment on this build', ...props } })
  flushSync()
  await nextTask()
  flushSync()
}

beforeEach(() => {
  Element.prototype.scrollTo = () => {}
  localStorage.clear()
  resetState()
  handleReady(ready)
  // newest first, as the server sends them; alternating authors, so each reply has its own name line
  const page = [reply(3), reply(2, { author_id: 'U1' }), reply(1)]
  invoke.mockReset()
  invoke.mockImplementation(async (cmd: string, args?: unknown) => {
    if (cmd === 'list_replies') return (args as { before: string | null }).before ? [] : page
    return undefined
  })
  windowFocus.focused = true
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})

describe('ThreadChat', () => {
  it('tags the root author\'s replies "author"', async () => {
    await show({ authorTag: 'U2' })
    expect(rowOf('reply 1').querySelector('.meta .author-tag')?.textContent).toBe('author')
    expect(rowOf('reply 3').querySelector('.meta .author-tag')?.textContent).toBe('author')
    expect(rowOf('reply 2').querySelector('.author-tag')).toBeNull()
  })

  it('tags no one without authorTag', async () => {
    await show()
    expect(document.querySelectorAll('.msg')).toHaveLength(3)
    expect(document.querySelector('.author-tag')).toBeNull()
  })

  it('lists the replies, uses its placeholder and sends into the thread', async () => {
    await show()
    expect(calls('list_replies')[0][1]).toEqual({ messageId: 'Q1', before: null })
    expect([...document.querySelectorAll('.msg .content')].map((e) => e.textContent?.trim())).toEqual(['reply 1', 'reply 2', 'reply 3'])
    expect(ui.openThreadId).toBe('Q1')
    const box = document.querySelector<HTMLElement>('[role="textbox"]')!
    expect(box.getAttribute('aria-placeholder')).toBe('Comment on this build')

    box.focus()
    box.replaceChildren(document.createTextNode('nice'))
    const p = positionAt(box, 4)
    const r = document.createRange()
    r.setStart(p.node, p.offset)
    r.collapse(true)
    getSelection()!.removeAllRanges()
    getSelection()!.addRange(r)
    box.dispatchEvent(new InputEvent('input', { bubbles: true }))
    flushSync()
    box.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true }))
    flushSync()
    expect(calls('send_reply')[0][1]).toMatchObject({ channelId: 'C1', rootId: 'Q1', content: 'nice', replyToId: null })
  })
})
