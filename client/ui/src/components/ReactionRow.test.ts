// @vitest-environment jsdom
import { flushSync, mount, tick, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Message } from '../lib/protocol/Message'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))

import ReactionRow from './ReactionRow.svelte'
import MessageItem from './MessageItem.svelte'
import { app } from '../lib/store.svelte'
import { emptyState } from '../lib/state'

const msg = (over: Partial<Message> = {}): Message => ({
  id: 'M1', channel_id: 'C1', author_id: 'U2', kind: 'normal', content: 'hi',
  reply_to_id: null, created_at: '2026-10-01T00:00:00.000Z', edited_at: null, deleted: false, mentions: [],
  reactions: [{ emoji: '👍', count: 2, me: true }], ...over,
})
const load = (m: Message) => {
  app.state = { ...emptyState(), messages: { C1: [m] } }
}
const cur = () => app.state.messages['C1'][0]
const chips = () => [...document.querySelectorAll('.chip:not(.add)')] as HTMLButtonElement[]

let cmp: ReturnType<typeof mount> | undefined
const onError = vi.fn()
function show(m: Message) {
  load(m)
  cmp = mount(ReactionRow, { target: document.body, props: { message: app.state.messages['C1'][0], meId: 'U1', nameOf: (id: string | null) => id ?? '', onError } })
  flushSync()
}
beforeEach(() => { invoke.mockReset(); invoke.mockResolvedValue(undefined); onError.mockClear() })
afterEach(() => { if (cmp) unmount(cmp); cmp = undefined; document.body.innerHTML = '' })

describe('ReactionRow', () => {
  it('clicking your highlighted chip removes your reaction', async () => {
    show(msg())
    chips()[0].click()
    await tick()
    expect(invoke).toHaveBeenCalledWith('remove_reaction', { messageId: 'M1', emoji: '👍' })
    expect(cur().reactions).toEqual([{ emoji: '👍', count: 1, me: false }])
  })

  it('a failed call restores the chip and reports the error', async () => {
    invoke.mockRejectedValueOnce('too many reactions')
    show(msg())
    chips()[0].click()
    await new Promise((r) => setTimeout(r, 0))
    expect(cur().reactions).toEqual([{ emoji: '👍', count: 2, me: true }])
    expect(onError).toHaveBeenCalledOnce()
  })

  it('deleted_custom_shows_name_chip', () => {
    const X1 = { id: 'X1', server_id: 'S1', name: 'party', hash: null, still_hash: null, animated: false, added_by: null, created_at: '2026-10-01T00:00:00.000Z' }
    show(msg({ reactions: [{ emoji: 'custom:X1', count: 3, me: false }, { emoji: 'custom:ZZ', count: 1, me: false }] }))
    app.state = { ...app.state, customEmoji: { X1 } } // X1 deleted (hash null); ZZ unknown
    flushSync()
    const gone = [...document.querySelectorAll('.gone')].map((e) => e.textContent)
    expect(gone).toEqual([':party:', ':custom:ZZ:'])
    expect(document.querySelector('.chip img')).toBeNull()
  })

  it('hover lists reactors once per count', async () => {
    invoke.mockResolvedValue(['U2', 'U3'])
    show(msg({ reactions: [{ emoji: '👍', count: 2, me: false }] }))
    const c = chips()[0]
    c.dispatchEvent(new Event('pointerenter'))
    await new Promise((r) => setTimeout(r, 0))
    flushSync()
    expect(document.querySelector('.tip')?.textContent).toBe('U2 and U3')
    c.dispatchEvent(new Event('pointerleave'))
    c.dispatchEvent(new Event('pointerenter'))
    await new Promise((r) => setTimeout(r, 0))
    expect(invoke.mock.calls.filter((x) => x[0] === 'list_reactors')).toHaveLength(1)
  })
})

describe('MessageItem reactions', () => {
  it('hidden_on_deleted_message', () => {
    load(msg({ deleted: true }))
    cmp = mount(MessageItem, { target: document.body, props: {
      message: cur(), name: 'sam', nameOf: () => 'sam', replyTo: null, grouped: false, mine: false,
      onReply: () => {}, onEdit: async () => {}, onDelete: () => {},
    } })
    flushSync()
    expect(document.querySelector('.reactions')).toBeNull()
    expect(document.querySelector('[aria-label="Add reaction"]')).toBeNull()
  })
})
