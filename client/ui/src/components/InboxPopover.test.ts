// @vitest-environment jsdom
// The inbox (#306): its count from Ready and events, the popover's tabs, rows, jump, Mark all read and paging.
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { InboxItem } from '../lib/protocol/InboxItem'
import type { InboxPage } from '../lib/protocol/InboxPage'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))

import InboxPopover from './InboxPopover.svelte'
import { closeInbox, inbox, noteInboxEvent, openInbox } from '../lib/inbox.svelte'
import { INBOX_PAGE } from '../lib/inbox'
import { app, clock, handleReady, resetState, ui } from '../lib/store.svelte'
import { testMe, testReady } from '../lib/testReady'
import { testItem as item, testMessage } from '../lib/testInbox'

const sam = { id: 'SAM', username: 'sam', display_name: 'Sam', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
const NOW = Date.parse('2026-11-12T10:05:00.000Z')

let pages: Record<string, InboxPage> = {}
let cmp: ReturnType<typeof mount> | undefined
const onClose = vi.fn()
const settle = async () => { for (let i = 0; i < 6; i++) await Promise.resolve(); flushSync() }
async function show() {
  openInbox()
  cmp = mount(InboxPopover, { target: document.body, props: { onClose } })
  flushSync()
  await settle()
}
const rows = () => [...document.querySelectorAll<HTMLButtonElement>('.item')]
const ids = () => rows().map((r) => r.querySelector('.said')?.textContent)
const mentionOf = (id: string, text: string, over: Partial<InboxItem> = {}) => item(id, over, { author_id: 'SAM', content: text, mentions: ['U1'], created_at: '2026-11-12T10:00:00.000Z' })

function ready(unread: number) {
  handleReady(testReady([
    { id: 'C1', server_id: 'S1', kind: 'text', name: 'general', position: 0 } as never,
    { id: 'Q1', server_id: 'S1', kind: 'quotes', name: 'quotes', position: 1 } as never,
    { id: 'K1', server_id: 'S1', kind: 'clips', name: 'clips', position: 2 } as never,
    { id: 'V9', server_id: 'S1', kind: 'voice', name: 'Lounge', position: 3 } as never,
  ], {
    members: [{ server_id: 'S1', members: [{ user: testMe, nickname: null }, { user: sam, nickname: null }] }],
    inbox_unread: unread,
  }))
}

beforeEach(() => {
  vi.useFakeTimers({ toFake: ['Date'] })
  vi.setSystemTime(NOW)
  clock.nowIso = new Date(NOW).toISOString()
  localStorage.clear()
  resetState()
  closeInbox()
  inbox.tab = 'all'
  pages = {
    all: { items: [mentionOf('M3', 'look @alex'), mentionOf('M2', 'agreed', { kind: 'reply', read: true, replied_to: { message_id: 'M0', author_id: 'U1', content: 'my take', deleted: false } })], unread: 1 },
    mentions: { items: [mentionOf('M3', 'look @alex')], unread: 1 },
  }
  invoke.mockReset()
  invoke.mockImplementation(async (cmd: string, args?: unknown) => {
    const a = args as { before?: string | null; mentions?: boolean } | undefined
    if (cmd === 'inbox') return a?.before ? { items: [], unread: 1 } : pages[a?.mentions ? 'mentions' : 'all']
    if (cmd === 'inbox_unread') return 1
    return undefined
  })
  ready(1)
  onClose.mockClear()
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
  vi.useRealTimers()
})

describe('the inbox (#306)', () => {
  it('Ready sets the count; events keep it', () => {
    expect(inbox.unread).toBe(1)
    noteInboxEvent({ t: 'InboxItemAdded', d: { user_id: 'U1', item: mentionOf('M9', 'x'), unread: 4 } })
    expect(inbox.unread).toBe(4)
    noteInboxEvent({ t: 'InboxUpdated', d: { user_id: 'U1', unread: 0, read: [], read_upto: null, removed: [] } })
    expect(inbox.unread).toBe(0)
    ready(7)
    expect(inbox.unread).toBe(7)
  })

  it('lists who, where, when, the preview with your mention picked out, and what a reply answered', async () => {
    await show()
    expect(invoke).toHaveBeenCalledWith('inbox', { before: null, mentions: false })
    expect(document.querySelector('[role=dialog]')?.getAttribute('aria-label')).toBe('Inbox')
    const [a, b] = rows()
    expect(a.classList.contains('unread')).toBe(true)
    expect(b.classList.contains('unread')).toBe(false)
    expect(a.querySelector('.author')?.textContent).toBe('Sam')
    expect(a.querySelector('.where')?.textContent).toBe('Main › #general')
    expect(a.querySelector('time')?.textContent).toBe('5m ago')
    expect(a.querySelector('mark.me')?.textContent).toBe('@alex')
    expect(a.querySelector('.said')?.textContent).toBe('look @alex')
    expect(a.getAttribute('aria-label')).toBe('Unread: mention from Sam in Main › #general')
    expect(b.querySelector('.snippet')?.textContent).toBe('Replying to you: my take')
    expect(b.getAttribute('aria-label')).toBe('reply from Sam in Main › #general')
  })

  it('the Mentions tab shows only mentions and is remembered on this device', async () => {
    await show()
    const tab = [...document.querySelectorAll<HTMLButtonElement>('[role=radio]')].find((b) => b.textContent === 'Mentions')!
    tab.click()
    await settle()
    expect(invoke).toHaveBeenCalledWith('inbox', { before: null, mentions: true })
    expect(ids()).toEqual(['look @alex'])
    expect(localStorage.getItem('pulse.inbox.tab')).toBe('mentions')
  })

  it('opening an item reads it and jumps to it; a reply in a quote opens its replies', async () => {
    await show()
    rows()[0].click()
    await settle()
    expect(invoke).toHaveBeenCalledWith('inbox_read', { messageId: 'M3' })
    expect(inbox.open).toBe(false)
    expect(inbox.unread).toBe(0)
    expect(inbox.lists.all?.items[0].read).toBe(true)
    expect(invoke).toHaveBeenCalledWith('list_messages_around', expect.objectContaining({ channelId: 'C1', messageId: 'M3' }))
    // a read one isn't read again
    invoke.mockClear()
    await show()
    rows()[1].click()
    expect(invoke).not.toHaveBeenCalledWith('inbox_read', expect.anything())
    // a quote's reply: its replies panel
    noteInboxEvent({ t: 'InboxItemAdded', d: { user_id: 'U1', item: item('R1', {}, { channel_id: 'Q1', thread_root_id: 'QT', author_id: 'SAM' }), unread: 1 } })
    await show()
    rows()[0].click()
    expect(ui.goTo).toBe('Q1')
    expect(ui.openReplies).toEqual({ channelId: 'Q1', rootId: 'QT' })
  })

  it('Mark all read reads everything up to the newest shown', async () => {
    await show()
    const mark = [...document.querySelectorAll('button')].find((b) => b.textContent === 'Mark all read')!
    mark.click()
    await settle()
    expect(invoke).toHaveBeenCalledWith('inbox_read_all', { upto: 'M3' })
    expect(inbox.unread).toBe(0)
    expect(rows().some((r) => r.classList.contains('unread'))).toBe(false)
    expect((mark as HTMLButtonElement).disabled).toBe(true)
  })

  it('live: new items on top, read elsewhere, read in the channel, deleted', async () => {
    await show()
    noteInboxEvent({ t: 'InboxItemAdded', d: { user_id: 'U1', item: mentionOf('M4', 'new one'), unread: 2 } })
    flushSync()
    expect(ids()).toEqual(['new one', 'look @alex', 'agreed'])
    noteInboxEvent({ t: 'InboxUpdated', d: { user_id: 'U1', unread: 1, read: ['M4'], read_upto: null, removed: [] } })
    flushSync()
    expect(rows()[0].classList.contains('unread')).toBe(false)
    noteInboxEvent({ t: 'ReadStateUpdated', d: { user_id: 'U1', channel_id: 'C1', last_read_message_id: 'M3', mentions: 0, unread: 0 } })
    flushSync()
    expect(rows()[1].classList.contains('unread')).toBe(false)
    noteInboxEvent({ t: 'InboxUpdated', d: { user_id: 'U1', unread: 0, read: [], read_upto: null, removed: ['M2'] } })
    flushSync()
    expect(ids()).toEqual(['new one', 'look @alex'])
  })

  it('a mute or unmute fetches the lists again (plain replies there come or go)', async () => {
    await show()
    invoke.mockClear()
    noteInboxEvent({ t: 'MutesChanged', d: { user_id: 'U1', mutes: [] } })
    await settle()
    expect(invoke).toHaveBeenCalledWith('inbox', { before: null, mentions: false })
    expect(rows()).toHaveLength(2)
  })

  it('a full page fetches the next one near the end; an empty inbox says so', async () => {
    pages.all = { items: Array.from({ length: INBOX_PAGE }, (_, i) => mentionOf(`M${String(100 - i).padStart(3, '0')}`, `n${i}`)), unread: 0 }
    await show()
    const list = document.querySelector<HTMLElement>('.list')!
    list.dispatchEvent(new Event('scroll'))
    await settle()
    expect(invoke).toHaveBeenCalledWith('inbox', { before: 'M071', mentions: false })
    expect(inbox.lists.all?.end).toBe(true)
    if (cmp) unmount(cmp)
    cmp = undefined
    pages.all = { items: [], unread: 0 }
    ready(0)
    await show()
    expect(document.querySelector('.none')?.textContent).toContain('Mentions and replies to you land here')
  })
})

describe("an inbox row's person menu (#135): who wrote it", () => {
  const menu = () => document.querySelector<HTMLElement>('[role=menu]')
  const itemIds = () => [...document.querySelectorAll<HTMLElement>('[role=menu] [data-item]')].map((i) => i.dataset.item)
  const key = (el: Element, k: string, init: KeyboardEventInit = {}) => {
    el.dispatchEvent(new KeyboardEvent('keydown', { key: k, bubbles: true, cancelable: true, ...init }))
    flushSync()
  }

  it('right-click a row: its author\'s menu, in the server it was said in', async () => {
    await show()
    const e = new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: 10, clientY: 10 })
    rows()[0].dispatchEvent(e)
    flushSync()
    expect(e.defaultPrevented).toBe(true)
    expect(menu()!.getAttribute('aria-label')).toBe('Actions for Sam')
    expect(itemIds()).toEqual(['profile', 'message', 'nick', 'private-nick', 'live-notify', 'appear-offline'])
  })

  it('Shift+F10 on a focused row opens it; Esc closes only the menu, focus back on the row', async () => {
    await show()
    const row = rows()[0]
    row.focus()
    key(row, 'F10', { shiftKey: true })
    expect(menu()).not.toBeNull()
    key(document.activeElement!, 'Escape')
    expect(menu()).toBeNull()
    expect(onClose).not.toHaveBeenCalled()
    expect(document.activeElement).toBe(row)
  })
})

describe('where an inbox item opens (#324)', () => {
  beforeEach(() => {
    Object.assign(ui, { goTo: null, notice: null, openChannelId: null, jump: null })
  })
  /** Show the popover with `it` as its one item, and click it. */
  async function openOnly(it: InboxItem) {
    pages = { all: { items: [it], unread: 1 }, mentions: { items: [it], unread: 1 } }
    inbox.lists = { all: null, mentions: null }
    await show()
    rows()[0].click()
    await settle()
  }
  const clip = testMessage('V1', { channel_id: 'K1', author_id: 'U1', mentions: [], content: 'clutch' })
  const clipReply = item('R5', { kind: 'reply' }, { channel_id: 'K1', thread_root_id: 'V1', mentions: [] })

  it('a reply to a clip fetches that clip and opens its panel', async () => {
    const before = testMessage('V0', { channel_id: 'K1' })
    const fallback = invoke.getMockImplementation()!
    invoke.mockImplementation(async (cmd, args) => (cmd === 'list_messages_around' ? [before, clip] : fallback(cmd, args)))
    await openOnly(clipReply)
    expect(invoke).toHaveBeenCalledWith('list_messages_around', { channelId: 'K1', messageId: 'V1' })
    expect(ui.goTo).toBe('K1')
    expect(ui.openClip).toEqual({ channelId: 'K1', clip })
    expect(ui.openReplies).toBeNull()
  })

  it('a reply to a clip already held here opens its panel at once', async () => {
    app.state = { ...app.state, clipFeeds: { k: { channelId: 'K1', filter: {}, items: [clip], start: true, loading: false } } }
    await openOnly(clipReply)
    expect(invoke).not.toHaveBeenCalledWith('list_messages_around', expect.anything())
    expect(ui.openClip).toEqual({ channelId: 'K1', clip })
  })

  it('a clip that mentions you opens its own panel', async () => {
    const mention = item('V2', {}, { channel_id: 'K1', content: 'gg @alex' })
    await openOnly(mention)
    expect(invoke).not.toHaveBeenCalledWith('list_messages_around', expect.anything())
    expect(ui.goTo).toBe('K1')
    expect(ui.openClip).toEqual({ channelId: 'K1', clip: mention.message })
  })

  it('a reply to a deleted clip says so and opens no panel', async () => {
    const fallback = invoke.getMockImplementation()!
    invoke.mockImplementation(async (cmd, args) => (cmd === 'list_messages_around' ? null : fallback(cmd, args)))
    await openOnly(clipReply)
    expect(ui.openClip).toBeNull()
    expect(ui.notice).toBe('That clip was deleted.')
  })

  it('a clip fetched after you went elsewhere opens nothing', async () => {
    let answer!: (v: unknown) => void
    const fallback = invoke.getMockImplementation()!
    invoke.mockImplementation((cmd, args) => (cmd === 'list_messages_around' ? new Promise((r) => (answer = r)) : fallback(cmd, args)))
    await openOnly(clipReply)
    ui.goTo = null
    ui.openChannelId = 'C1'
    answer([clip])
    await settle()
    expect(ui.openClip).toBeNull()
  })

  it('a reply to a quote opens its replies; a voice channel chat mention jumps there', async () => {
    await openOnly(item('R6', { kind: 'reply' }, { channel_id: 'Q1', thread_root_id: 'QT' }))
    expect(ui.goTo).toBe('Q1')
    expect(ui.openReplies).toEqual({ channelId: 'Q1', rootId: 'QT' })
    expect(ui.openClip).toBeNull()
    unmount(cmp!)
    cmp = undefined
    invoke.mockClear()
    await openOnly(item('M7', {}, { channel_id: 'V9' }))
    expect(invoke).toHaveBeenCalledWith('list_messages_around', { channelId: 'V9', messageId: 'M7' })
  })

  it('an item in a chat no longer here says so', async () => {
    await openOnly(item('M8', {}, { channel_id: 'GONE', thread_root_id: 'X' }))
    expect(ui.notice).toBe('That channel was deleted.')
    expect(ui.openReplies).toBeNull()
  })
})
