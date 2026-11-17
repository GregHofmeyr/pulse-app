// @vitest-environment jsdom
// Save message (#315) in a message's menu: saves a private bookmark; one you saved offers Remove from saved.
import { flushSync, mount, tick, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Message } from '../lib/protocol/Message'
import type { MessageBookmark } from '../lib/protocol/MessageBookmark'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))

import MessageItem from './MessageItem.svelte'
import { app, ui } from '../lib/store.svelte'
import { emptyState } from '../lib/state'
import { bookmarks } from '../lib/bookmarks.svelte'

const msg = (over: Partial<Message> = {}): Message => ({
  id: 'M1', channel_id: 'C1', author_id: 'U2', kind: 'normal', content: 'the gate code is 4321',
  reply_to_id: null, created_at: '2026-11-16T09:00:00.000Z', edited_at: null, deleted: false, mentions: [],
  reactions: [], thread_root_id: null, quote: null, qoty_card: null, clip: null, embeds: [], ...over,
})
const made: MessageBookmark = {
  id: 'B1', message_id: 'M1', note: null, created_at: '2026-11-16T10:00:00.000Z',
  message: { channel_id: 'C1', thread_root_id: null, author_id: 'U2', content: 'the gate code is 4321', created_at: '2026-11-16T09:00:00.000Z' },
}

let cmp: ReturnType<typeof mount> | undefined
function show(m: Message) {
  app.state = {
    ...emptyState(),
    channels: { C1: { id: 'C1', server_id: 'S1', kind: 'text', name: 'general', position: 0 } },
    messages: { C1: [m] },
  }
  cmp = mount(MessageItem, { target: document.body, props: {
    message: m, serverId: null, name: 'sam', nameOf: (id: string | null) => (id === 'U2' ? 'sam' : 'alex'), replyTo: null,
    grouped: false, mine: false, onReply: () => {}, onEdit: async () => {}, onDelete: async () => {}, meId: 'U1',
  } })
  flushSync()
}
const btn = (label: string) => document.querySelector<HTMLButtonElement>(`button[aria-label="${label}"]`)
const menu = () => document.querySelector<HTMLElement>('[role=menu][aria-label="Message actions"]')
const items = () => [...(menu()?.querySelectorAll<HTMLButtonElement>(':scope > [role=menuitem]') ?? [])]
const item = (label: string) => items().find((b) => b.textContent?.trim() === label)
async function settle() { for (let i = 0; i < 5; i++) await Promise.resolve(); await tick() }

beforeEach(() => {
  invoke.mockReset()
  invoke.mockImplementation(async (cmd: string) => (cmd === 'save_message' ? made : undefined))
  bookmarks.list = []
  ui.notice = null
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})

describe('Save message (#315)', () => {
  it('saves the message, privately, and says where to find it', async () => {
    show(msg())
    btn('More')!.click(); flushSync()
    item('Save message')!.click()
    await settle()
    expect(invoke).toHaveBeenCalledWith('save_message', { messageId: 'M1', note: null })
    expect(bookmarks.list.map((b) => b.id)).toEqual(['B1'])
    expect(ui.notice).toContain('Saved')
  })

  it('a saved message offers Remove from saved instead', async () => {
    bookmarks.list = [made]
    show(msg())
    btn('More')!.click(); flushSync()
    expect(item('Save message')).toBeUndefined()
    item('Remove from saved')!.click()
    await settle()
    expect(invoke).toHaveBeenCalledWith('delete_bookmark', { id: 'B1' })
    expect(bookmarks.list).toEqual([])
  })
})
