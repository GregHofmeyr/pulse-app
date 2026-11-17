// @vitest-environment jsdom
// The inbox's Threads tab (#326): threads you started or replied in with replies you haven't seen, fetched on Ready
// and again when a reply lands in one you watch; each counts toward the badge (in the server's count, #419); a click opens the thread and counts
// it seen; your own reply, or your seen point, takes it away; Mark all read sees them all.
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { InboxThread } from '../lib/protocol/InboxThread'
import type { InboxThreads } from '../lib/protocol/InboxThreads'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))

import InboxPopover from './InboxPopover.svelte'
import { closeInbox, inbox, inboxBadge, markAllRead, openInbox, setTab } from '../lib/inbox.svelte'
import { noteThreadsEvent, REFETCH_MS, resetThreads, threads } from '../lib/inboxThreads.svelte'
import { clock, handleReady, resetState, ui } from '../lib/store.svelte'
import { testMe, testReady } from '../lib/testReady'
import { testMessage } from '../lib/testInbox'

const sam = { id: 'SAM', username: 'sam', display_name: 'Sam', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
const NOW = Date.parse('2026-11-12T10:05:00.000Z')
const thread = (root: string, latest: string, n: number, started = true): InboxThread => ({
  root: { message_id: root, author_id: 'U1', content: 'the quote', deleted: false },
  channel_id: 'Q1',
  latest: testMessage(latest, { channel_id: 'Q1', thread_root_id: root, content: 'ha, true', mentions: [] }),
  new_replies: n,
  started,
})

let answer: InboxThreads = { threads: [], watching: [] }
let cmp: ReturnType<typeof mount> | undefined
const settle = async () => { for (let i = 0; i < 6; i++) await Promise.resolve(); flushSync() }

beforeEach(async () => {
  vi.useFakeTimers({ toFake: ['Date', 'setTimeout', 'clearTimeout'] })
  vi.setSystemTime(NOW)
  clock.nowIso = new Date(NOW).toISOString()
  localStorage.clear()
  resetState()
  resetThreads()
  closeInbox()
  answer = { threads: [thread('R1', 'X2', 2)], watching: ['R1', 'R2'] }
  invoke.mockReset()
  invoke.mockImplementation(async (cmd: string) => {
    if (cmd === 'inbox_threads') return answer
    // one item and the one thread: the server counts both (#419)
    if (cmd === 'inbox') return { items: [], unread: 2 }
    if (cmd === 'inbox_unread') return 2
    return undefined
  })
  handleReady(testReady([{ id: 'Q1', server_id: 'S1', kind: 'quotes', name: 'quotes', position: 1 } as never], {
    members: [{ server_id: 'S1', members: [{ user: testMe, nickname: null }, { user: sam, nickname: null }] }],
    inbox_unread: 2,
  }))
  await settle()
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
  vi.useRealTimers()
})

async function show() {
  openInbox()
  setTab('threads')
  cmp = mount(InboxPopover, { target: document.body, props: { onClose: () => {} } })
  flushSync()
  await settle()
}

describe('inbox Threads (#326)', () => {
  it("Ready fetches them; the badge is the server's count, which has them already (#419)", () => {
    expect(invoke).toHaveBeenCalledWith('inbox_threads', undefined)
    expect(threads.list.map((t) => t.root.message_id)).toEqual(['R1'])
    expect(inboxBadge()).toBe(2)
  })

  it('the tab lists each thread: newest reply, how many are new, and what it is on', async () => {
    await show()
    const row = document.querySelector<HTMLButtonElement>('.item')!
    expect(row.querySelector('.author')!.textContent).toBe('Sam')
    expect(row.querySelector('.where')!.textContent).toBe('Main › #quotes › replies')
    expect(row.querySelector('.snippet')!.textContent!.trim()).toBe('2 new replies · your quote: the quote')
    expect(row.querySelector('.said')!.textContent).toBe('ha, true')
    expect(document.querySelector('h2 .count')!.textContent).toBe('2')
  })

  it('a click opens the thread and counts it seen', async () => {
    await show()
    document.querySelector<HTMLButtonElement>('.item')!.click()
    await settle()
    expect(ui.openReplies).toEqual({ channelId: 'Q1', rootId: 'R1' })
    expect(invoke).toHaveBeenCalledWith('mark_replies_seen', { rootId: 'R1', messageId: 'X2' })
    expect(threads.list).toEqual([])
    expect(inbox.open).toBe(false)
  })

  it("a reply in a watched thread fetches again (once, a moment later); one in a thread you aren't part of doesn't", async () => {
    invoke.mockClear()
    noteThreadsEvent({ t: 'MessageCreated', d: { message: testMessage('X9', { channel_id: 'Q1', thread_root_id: 'R7', mentions: [] }), nonce: null } })
    vi.advanceTimersByTime(REFETCH_MS)
    expect(invoke).not.toHaveBeenCalled()
    noteThreadsEvent({ t: 'MessageCreated', d: { message: testMessage('X3', { channel_id: 'Q1', thread_root_id: 'R2', mentions: [] }), nonce: null } })
    noteThreadsEvent({ t: 'MessageCreated', d: { message: testMessage('X4', { channel_id: 'Q1', thread_root_id: 'R2', mentions: [] }), nonce: null } })
    vi.advanceTimersByTime(REFETCH_MS)
    expect(invoke.mock.calls.filter(([c]) => c === 'inbox_threads')).toHaveLength(1)
  })

  it('your own reply, or your seen point past its newest reply, takes a thread away; your reply makes you watch it', () => {
    noteThreadsEvent({ t: 'MessageCreated', d: { message: testMessage('X5', { author_id: 'U1', channel_id: 'Q1', thread_root_id: 'R9', mentions: [] }), nonce: null } })
    expect(threads.watching).toContain('R9')
    noteThreadsEvent({ t: 'ThreadSeen', d: { user_id: 'U1', channel_id: 'Q1', root_id: 'R1', last_seen_id: 'X2', mentions_cleared: 0 } } as never)
    expect(threads.list).toEqual([])
  })

  it('Mark all read sees every thread, before it reads the rest', async () => {
    await markAllRead()
    expect(invoke).toHaveBeenCalledWith('mark_replies_seen', { rootId: 'R1', messageId: 'X2' })
    expect(threads.list).toEqual([])
    const order = invoke.mock.calls.map(([c]) => c).filter((c) => c === 'mark_replies_seen' || c === 'inbox_read_all')
    expect(order).toEqual(['mark_replies_seen', 'inbox_read_all'])
  })
})
