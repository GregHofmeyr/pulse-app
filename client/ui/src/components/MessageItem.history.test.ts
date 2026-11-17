// @vitest-environment jsdom
// Edit history (#301): "(edited)" is a button that opens the message's versions, newest first, with their times; the
// message menu's Edit history (#415) opens the same dialog.
import { flushSync, mount, tick, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Message } from '../lib/protocol/Message'
import type { MessageVersion } from '../lib/protocol/MessageVersion'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))

import MessageItem from './MessageItem.svelte'
import { app } from '../lib/store.svelte'
import { emptyState } from '../lib/state'

const msg = (over: Partial<Message> = {}): Message => ({
  id: 'M1', channel_id: 'C1', author_id: 'U2', kind: 'normal', content: 'see you at 8',
  reply_to_id: null, created_at: '2026-11-16T09:00:00.000Z', edited_at: '2026-11-16T09:05:00.000Z', deleted: false, mentions: [],
  reactions: [], thread_root_id: null, quote: null, qoty_card: null, clip: null, embeds: [], ...over,
})
const versions: MessageVersion[] = [
  { content: 'see you at 8', written_at: '2026-11-16T09:05:00.000Z' },
  { content: 'see you at 7', written_at: '2026-11-16T09:02:00.000Z' },
  { content: 'see you at 6', written_at: '2026-11-16T09:00:00.000Z' },
]

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
async function settle() { for (let i = 0; i < 5; i++) await Promise.resolve(); await tick() }
const dialog = () => document.querySelector<HTMLElement>('[role=dialog][aria-label="Edit history"]')

beforeEach(() => {
  invoke.mockReset()
  invoke.mockImplementation(async (cmd: string) => (cmd === 'message_edits' ? versions : undefined))
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})

describe('edit history (#301)', () => {
  it('(edited) opens the versions, newest first, the current and the original marked', async () => {
    show(msg())
    const edited = document.querySelector<HTMLButtonElement>('button.edited')!
    expect(edited.textContent).toBe('(edited)')
    edited.click(); flushSync()
    await settle()
    expect(invoke).toHaveBeenCalledWith('message_edits', { messageId: 'M1' })
    const rows = [...dialog()!.querySelectorAll('.version')]
    expect(rows.map((r) => r.querySelector('.text')!.textContent)).toEqual(['see you at 8', 'see you at 7', 'see you at 6'])
    expect(rows[0].querySelector('.when')!.textContent).toContain('current')
    expect(rows[2].querySelector('.when')!.textContent).toContain('original')
  })

  it('Escape closes it; a refusal says why', async () => {
    show(msg())
    document.querySelector<HTMLButtonElement>('button.edited')!.click(); flushSync()
    await settle()
    dialog()!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true })); flushSync()
    expect(dialog()).toBeNull()
    invoke.mockImplementation(async () => { throw { kind: 'NotFound', message: 'not found' } })
    document.querySelector<HTMLButtonElement>('button.edited')!.click(); flushSync()
    await settle()
    expect(dialog()!.querySelector('[role=alert]')).not.toBeNull()
  })

  it('a message never edited has no (edited)', () => {
    show(msg({ edited_at: null }))
    expect(document.querySelector('.edited')).toBeNull()
  })

  it('the message menu has Edit history on an edited message, opening the same versions (#415)', async () => {
    show(msg())
    document.querySelector<HTMLButtonElement>('button[aria-label=More]')!.click(); flushSync()
    const item = [...document.querySelectorAll<HTMLButtonElement>('[role=menu] [role=menuitem]')].find((b) => b.textContent === 'Edit history')!
    expect(item.getAttribute('aria-haspopup')).toBe('dialog')
    item.click(); flushSync()
    await settle()
    expect(document.querySelector('[role=menu][aria-label="Message actions"]')).toBeNull()
    expect(invoke).toHaveBeenCalledWith('message_edits', { messageId: 'M1' })
    expect(dialog()!.querySelectorAll('.version')).toHaveLength(3)
  })

  it('a message never edited has no Edit history in its menu', () => {
    show(msg({ edited_at: null }))
    document.querySelector('.content')!.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: 50, clientY: 50 }))
    flushSync()
    const items = [...document.querySelectorAll('[role=menu] [role=menuitem]')].map((b) => b.textContent)
    expect(items.length).toBeGreaterThan(0)
    expect(items).not.toContain('Edit history')
  })
})
