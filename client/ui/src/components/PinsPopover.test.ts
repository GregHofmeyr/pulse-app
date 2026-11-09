// @vitest-environment jsdom
// The pins popover (#178): a chat's pins, newest first, each with Jump and Unpin; live from events.
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Message } from '../lib/protocol/Message'
import type { PinnedMessage } from '../lib/protocol/PinnedMessage'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))

import PinsPopover from './PinsPopover.svelte'
import { app, handleReady, resetState } from '../lib/store.svelte'
import { applyEvent } from '../lib/state'
import { testMe, testReady } from '../lib/testReady'

const sam = { id: 'U2', username: 'sam', display_name: 'sam', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
const NOW = Date.parse('2026-11-09T10:00:00.000Z')
const msg = (id: string, content: string, author_id = 'U1'): Message => ({
  id, channel_id: 'C1', author_id, kind: 'normal', content, reply_to_id: null, created_at: '2026-11-09T09:00:00.000Z',
  edited_at: null, deleted: false, mentions: [], reactions: [], thread_root_id: null, quote: null, qoty_card: null, clip: null, embeds: [],
})
const pinned = (m: Message, by: string, expires_at: string | null): PinnedMessage => {
  const pin = { pinned_by: by, pinned_at: '2026-11-09T09:30:00.000Z', expires_at }
  return { message: { ...m, pin }, pin }
}
const list = [
  pinned(msg('M2', 'a link', 'U2'), 'U1', '2026-11-09T13:00:00.000Z'),
  pinned(msg('M1', 'plan for tonight'), 'U2', null),
]

let cmp: ReturnType<typeof mount> | undefined
const onJump = vi.fn()
const onClose = vi.fn()
const settle = async () => { for (let i = 0; i < 5; i++) await Promise.resolve(); flushSync() }
async function show() {
  cmp = mount(PinsPopover, { target: document.body, props: { channelId: 'C1', serverId: 'S1', onJump, onClose } })
  flushSync()
  await settle()
}
const rows = () => [...document.querySelectorAll<HTMLElement>('.pin')]
const button = (row: HTMLElement, label: string) => [...row.querySelectorAll('button')].find((b) => b.textContent?.trim() === label)!

beforeEach(() => {
  vi.useFakeTimers({ toFake: ['Date'] })
  vi.setSystemTime(NOW)
  resetState()
  handleReady(testReady([{ id: 'C1', server_id: 'S1', kind: 'text', name: 'general', position: 0 }], {
    members: [{ server_id: 'S1', members: [{ user: testMe, nickname: null }, { user: sam, nickname: null }] }],
  }))
  invoke.mockReset()
  invoke.mockImplementation(async (cmd: string) => (cmd === 'list_pins' ? list : undefined))
  onJump.mockClear()
  onClose.mockClear()
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
  vi.useRealTimers()
})

describe('PinsPopover (#178)', () => {
  it('lists the pins newest first: author, preview, who pinned it and the time left', async () => {
    await show()
    expect(invoke).toHaveBeenCalledWith('list_pins', { channelId: 'C1' })
    expect(document.querySelector('[role=dialog]')?.getAttribute('aria-label')).toBe('Pinned messages')
    const [a, b] = rows()
    expect(a.querySelector('.author')?.textContent).toBe('sam')
    expect(a.querySelector('.said')?.textContent).toBe('a link')
    expect(a.querySelector('.note')?.textContent).toBe('Pinned by alex · 3 hours left')
    expect(b.querySelector('.said')?.textContent).toBe('plan for tonight')
    expect(b.querySelector('.note')?.textContent).toBe('Pinned by sam')
  })

  it('Jump asks to show the message; Unpin unpins it', async () => {
    await show()
    button(rows()[1], 'Jump').click()
    expect(onJump).toHaveBeenCalledWith('M1')
    button(rows()[0], 'Unpin').click()
    await settle()
    expect(invoke).toHaveBeenCalledWith('unpin_message', { messageId: 'M2' })
  })

  it('a failed unpin says why', async () => {
    await show()
    invoke.mockImplementation(async (c: string) => { if (c === 'unpin_message') throw 'nope' })
    button(rows()[0], 'Unpin').click()
    await settle()
    expect(document.querySelector('[role=alert]')?.textContent).toBe('nope')
  })

  it('follows pins and unpins from anyone, live', async () => {
    await show()
    const p = pinned(msg('M3', 'new one'), 'U2', null)
    app.state = applyEvent(app.state, { t: 'MessagePinned', d: { channel_id: 'C1', pinned: p } }, 0)
    flushSync()
    expect(rows().map((r) => r.querySelector('.said')?.textContent)).toEqual(['new one', 'a link', 'plan for tonight'])
    app.state = applyEvent(app.state, { t: 'MessageUnpinned', d: { channel_id: 'C1', message_id: 'M2' } }, 0)
    flushSync()
    expect(rows().map((r) => r.querySelector('.said')?.textContent)).toEqual(['new one', 'plan for tonight'])
  })

  it('fetches again after a reconnect forgot the list, and stays live', async () => {
    await show()
    expect(rows()).toHaveLength(2)
    invoke.mockClear()
    app.state = { ...app.state, pins: {} } // what Ready does
    flushSync()
    await settle()
    expect(invoke).toHaveBeenCalledWith('list_pins', { channelId: 'C1' })
    expect(rows()).toHaveLength(2)
  })

  it('has a friendly line when nothing is pinned', async () => {
    invoke.mockImplementation(async (cmd: string) => (cmd === 'list_pins' ? [] : undefined))
    await show()
    expect(rows()).toHaveLength(0)
    expect(document.querySelector('.none')?.textContent).toBe('Nothing pinned yet. Pin a message from its menu to keep it here.')
  })
})
